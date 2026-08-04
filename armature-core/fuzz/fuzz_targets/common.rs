//! Small helpers shared between fuzz targets.
//!
//! `cargo-fuzz` targets are each their own `[[bin]]` (see
//! `armature-core/fuzz/Cargo.toml`) rather than crate modules, so this file is
//! pulled in via `#[path = "common.rs"] mod common;` from whichever targets
//! need it, instead of being a `lib.rs`.

/// Build a raw HTTP/1.x request-line buffer: `"{method} {target} HTTP/1.{version}\r\n"`.
///
/// Shared by the header-parsing and URL-parsing fuzz targets, which both
/// need to synthesize a plausible request line around fuzzer-controlled
/// data to reach deeper into the real parsers.
#[allow(dead_code)]
pub fn synth_request_line(method: &str, target: &str, version_digit: u8) -> Vec<u8> {
    let mut line = Vec::with_capacity(method.len() + target.len() + 16);
    line.extend_from_slice(method.as_bytes());
    line.push(b' ');
    line.extend_from_slice(target.as_bytes());
    line.extend_from_slice(b" HTTP/1.");
    line.push(b'0' + (version_digit % 10));
    line.extend_from_slice(b"\r\n");
    line
}
