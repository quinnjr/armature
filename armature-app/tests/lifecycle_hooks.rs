//! End-to-end regression test for the Info finding: `run()`'s
//! `on_bootstrap` hook was invoked with `let _ = hook.call(...)`,
//! discarding any error — a failing bootstrap hook was swallowed and the
//! server started anyway with no diagnostic.
//!
//! `armature-app/src/runner.rs:78`
//!
//! `fire_bootstrap_hook`/`fire_shutdown_hook`'s exact behavioral contract
//! (bootstrap propagates+aborts, shutdown only logs) has focused unit
//! coverage in `src/runner.rs`'s `#[cfg(test)] mod tests`, since
//! `on_shutdown` is unreachable through the public `run()` API today (see
//! the note in the module doc there). This is the end-to-end proof for
//! the bootstrap half, which *is* reachable through `run()` because it
//! fires before the network listener is ever bound.

use armature_app::{RunConfig, run};
use std::io::Write;
use std::time::Duration;
use tempfile::NamedTempFile;

fn write_script(contents: &str) -> NamedTempFile {
    let mut file = NamedTempFile::with_suffix(".rhai").expect("create temp script file");
    file.write_all(contents.as_bytes()).expect("write script");
    file.flush().expect("flush script");
    file
}

#[tokio::test]
async fn failing_bootstrap_hook_aborts_startup_with_a_clear_error() {
    let script = write_script(
        r#"
            let app_module = create_module("AppModule");
            let app = create_app(app_module);
            app.on_bootstrap(|| {
                throw "bootstrap exploded";
            });
        "#,
    );

    let config = RunConfig {
        port: Some(0),
        host: Some("127.0.0.1".to_string()),
    };

    // Bounded with a timeout: the pre-fix behavior swallowed the hook's
    // error and proceeded to build the router and bind the listener,
    // which never returns — so a hang here is itself evidence of the bug.
    let outcome = tokio::time::timeout(Duration::from_secs(5), run(script.path(), config)).await;

    let result = outcome.unwrap_or_else(|_| {
        panic!(
            "run() should return promptly when on_bootstrap fails, not hang (a hang would mean \
             the hook's error was swallowed and the server started anyway)"
        )
    });

    let err = result.expect_err("a failing on_bootstrap hook must abort startup, not be swallowed");
    assert!(
        err.to_string().contains("bootstrap"),
        "error should mention the failing bootstrap hook, got: {err}"
    );
}

#[tokio::test]
async fn succeeding_bootstrap_hook_does_not_block_startup() {
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
        listener.local_addr().unwrap().port()
    };

    let script = write_script(
        r#"
            let app_module = create_module("AppModule");
            let app = create_app(app_module);
            app.on_bootstrap(|| {
                log_info("bootstrap ran fine");
            });
        "#,
    );
    let script_path = script.path().to_path_buf();

    let config = RunConfig {
        port: Some(port),
        host: Some("127.0.0.1".to_string()),
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
        "a succeeding on_bootstrap hook must not prevent the server from starting"
    );
}
