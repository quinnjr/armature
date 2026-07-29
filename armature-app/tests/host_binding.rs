//! Regression test for the Warning finding: the runner computes `host`
//! from `RunConfig`/`app_def.host` (default `0.0.0.0`) and logs it, and
//! scripts can set it via `listen_host(port, host)`, but the value was
//! discarded — `Application::listen(port)` hardcoded the bind address to
//! `[0, 0, 0, 0]`. Any non-default host (e.g. `127.0.0.1`) was silently
//! ignored.
//!
//! `armature-app/src/runner.rs:64`
//!
//! `resolve_bind_addr`'s pure IP-parsing logic has focused unit coverage
//! in `src/runner.rs`'s `#[cfg(test)] mod tests`. These are the
//! end-to-end tests: an ephemeral bind that's actually connectable at the
//! configured host/port, and a full `run()` call proving an unparseable
//! host surfaces a clear error instead of hanging (which is what the
//! pre-fix code effectively did: `host` was ignored, so the server always
//! started on `0.0.0.0` regardless of what — if anything — `host` parsed
//! to).

use armature_app::{RunConfig, run};
use std::io::Write;
use std::net::TcpListener;
use std::time::Duration;
use tempfile::NamedTempFile;

fn write_script(contents: &str) -> NamedTempFile {
    let mut file = NamedTempFile::with_suffix(".rhai").expect("create temp script file");
    file.write_all(contents.as_bytes()).expect("write script");
    file.flush().expect("flush script");
    file
}

/// Reserve a free TCP port on 127.0.0.1 by binding to port 0 and reading
/// back the OS-assigned port, then releasing it immediately. There's an
/// unavoidable small race between releasing it here and the app under
/// test binding it a moment later, but this is the standard technique for
/// handing a real, known-free port to a server under test instead of
/// hardcoding a fixed port that might collide with something else.
fn reserve_free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral port");
    listener.local_addr().expect("read local addr").port()
}

const MINIMAL_APP: &str = r#"
    let app_module = create_module("AppModule");
    let app = create_app(app_module);
"#;

#[tokio::test]
async fn host_knob_is_honored_server_is_reachable_on_the_configured_host_and_port() {
    let port = reserve_free_port();
    let script = write_script(MINIMAL_APP);
    let script_path = script.path().to_path_buf();

    let config = RunConfig {
        port: Some(port),
        host: Some("127.0.0.1".to_string()),
    };

    let handle = tokio::spawn(async move {
        let _ = run(&script_path, config).await;
    });

    // Poll until the server accepts connections, bounded by a deadline.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut connected = false;
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    handle.abort();

    assert!(
        connected,
        "expected the server to become connectable on 127.0.0.1:{port} — the exact host/port \
         configured via RunConfig — within 5s"
    );
}

#[tokio::test]
async fn unparseable_host_returns_a_clear_error_promptly_instead_of_starting_anyway() {
    let script = write_script(MINIMAL_APP);
    let config = RunConfig {
        port: Some(0),
        host: Some("not-a-valid-host".to_string()),
    };

    // Bounded with a timeout: the pre-fix behavior discarded `host`
    // entirely and started the server anyway (bound to 0.0.0.0), which
    // never returns — so a hang here is itself evidence of the bug.
    let outcome = tokio::time::timeout(Duration::from_secs(5), run(script.path(), config)).await;

    let result = outcome.unwrap_or_else(|_| {
        panic!(
            "run() should return promptly with an error for an unparseable host, not hang \
             (a hang would mean the host was silently discarded and the server started anyway)"
        )
    });

    let err = result.expect_err("an unparseable host must not silently succeed");
    assert!(
        err.to_string().contains("not-a-valid-host"),
        "error should name the offending host, got: {err}"
    );
}

#[tokio::test]
async fn default_host_still_binds_all_interfaces_when_unspecified() {
    // No host override anywhere (config.host is None, script never calls
    // listen_host) — the documented default of 0.0.0.0 must still work.
    let port = reserve_free_port();
    let script = write_script(MINIMAL_APP);
    let script_path = script.path().to_path_buf();

    let config = RunConfig {
        port: Some(port),
        host: None,
    };

    let handle = tokio::spawn(async move {
        let _ = run(&script_path, config).await;
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut connected = false;
    while tokio::time::Instant::now() < deadline {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    handle.abort();

    assert!(
        connected,
        "the default 0.0.0.0 bind should still be reachable via 127.0.0.1:{port}"
    );
}
