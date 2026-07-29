//! Mailgun must honor a `Retry-After` on 429.
//!
//! It hardcoded 60 and ignored the header, while SendGrid parsed it. That is
//! not cosmetic: `EmailQueueWorker::calculate_backoff` keys the *entire* retry
//! schedule for a throttled job off `MailError::retry_after()`, so a Mailgun
//! `Retry-After: 300` was retried after 60s and re-throttled, burning quota on
//! every attempt.
//!
//! Driven against a real local HTTP server rather than a unit test on the parse
//! helper, so the header actually has to survive the transport.

#![cfg(feature = "mailgun")]

use armature_mail::{Email, MailError, MailgunConfig, MailgunTransport, Transport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serve exactly one request, replying with `status` and the given headers.
///
/// Reads the request in full (headers, then `Content-Length` bytes) before
/// replying: answering early makes reqwest's in-flight multipart write race the
/// connection close, which shows up as a flaky `Network` error instead of the
/// status under test.
async fn serve_once(listener: TcpListener, status_line: &'static str, headers: &'static str) {
    let (mut socket, _) = listener.accept().await.unwrap();

    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];

    // Request head.
    let header_end = loop {
        let n = socket.read(&mut chunk).await.unwrap();
        if n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_ascii_lowercase();
    let content_length: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    // Request body.
    while buf.len() < header_end + content_length {
        let n = socket.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }

    let response = format!("HTTP/1.1 {status_line}\r\n{headers}Content-Length: 0\r\n\r\n");
    socket.write_all(response.as_bytes()).await.unwrap();
    socket.flush().await.unwrap();
}

fn test_email() -> Email {
    Email::new()
        .from("sender@example.com")
        .to("recipient@example.com")
        .subject("Test")
        .text("Hello")
}

/// Send one email against a local server returning `status_line`/`headers`.
async fn send_against(status_line: &'static str, headers: &'static str) -> MailError {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(serve_once(listener, status_line, headers));

    // Loopback http is explicitly permitted by the endpoint validation, which
    // exists to stop the API key going out in cleartext to a *remote* host.
    let transport = MailgunTransport::new(
        MailgunConfig::new("key-test", "example.com").base_url(format!("http://127.0.0.1:{port}")),
    )
    .unwrap();

    let err = transport
        .send(&test_email())
        .await
        .expect_err("a 429 must surface as an error");
    server.await.unwrap();
    err
}

#[tokio::test]
async fn a_429_carries_the_servers_retry_after() {
    let err = send_against("429 Too Many Requests", "Retry-After: 30\r\n").await;

    assert!(
        matches!(err, MailError::RateLimited(30)),
        "Retry-After was ignored: {err:?}"
    );
    assert_eq!(err.retry_after(), Some(std::time::Duration::from_secs(30)));
}

/// A larger value must survive intact — this is the case the hardcoded 60
/// actively broke, by retrying five times sooner than the provider asked.
#[tokio::test]
async fn a_long_retry_after_is_not_truncated_to_the_default() {
    let err = send_against("429 Too Many Requests", "Retry-After: 300\r\n").await;
    assert!(matches!(err, MailError::RateLimited(300)), "{err:?}");
}

/// No header: fall back to the previous fixed 60 rather than guessing.
#[tokio::test]
async fn a_429_without_the_header_falls_back_to_sixty_seconds() {
    let err = send_against("429 Too Many Requests", "").await;
    assert!(matches!(err, MailError::RateLimited(60)), "{err:?}");
}

/// A rate-limit error must be retryable, or the parsed delay is never used.
#[tokio::test]
async fn a_rate_limited_send_is_retryable() {
    let err = send_against("429 Too Many Requests", "Retry-After: 30\r\n").await;
    assert!(err.is_retryable(), "{err:?}");
}
