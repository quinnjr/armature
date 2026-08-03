//! Fuzz target for `HttpRequest` construction and its accessors.
//!
//! `HttpRequest` is built on every served request, and the accessors fuzzed
//! here — `path_only`, `query`/`query_param`, `param`, header lookup — are the
//! ones handlers and middleware actually call, so arbitrary request targets
//! reach the real target-splitting and query-parsing code rather than a
//! stand-in. Bodies and headers are fuzzer-controlled to cover non-UTF-8 and
//! degenerate header names.

#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

use armature_core::HttpMethod;
use armature_core::http::HttpRequest;
use bytes::Bytes;

/// Arbitrary HTTP request for fuzzing.
#[derive(Debug, Arbitrary)]
struct FuzzRequest {
    method: FuzzMethod,
    path: String,
    query: Option<String>,
    headers: Vec<(String, String)>,
    params: Vec<(String, Vec<u8>)>,
    body: Vec<u8>,
}

#[derive(Debug, Arbitrary)]
enum FuzzMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
    Connect,
    Trace,
    Query,
    /// An unrecognized token, carried as `Method::Other` rather than rejected.
    Custom(String),
}

impl FuzzMethod {
    fn as_str(&self) -> &str {
        match self {
            FuzzMethod::Get => HttpMethod::GET.as_str(),
            FuzzMethod::Post => HttpMethod::POST.as_str(),
            FuzzMethod::Put => HttpMethod::PUT.as_str(),
            FuzzMethod::Delete => HttpMethod::DELETE.as_str(),
            FuzzMethod::Patch => HttpMethod::PATCH.as_str(),
            FuzzMethod::Head => HttpMethod::HEAD.as_str(),
            FuzzMethod::Options => HttpMethod::OPTIONS.as_str(),
            FuzzMethod::Connect => "CONNECT",
            FuzzMethod::Trace => "TRACE",
            FuzzMethod::Query => HttpMethod::QUERY.as_str(),
            FuzzMethod::Custom(s) => s.as_str(),
        }
    }
}

fuzz_target!(|data: FuzzRequest| {
    if data.path.len() > 10_000 || data.body.len() > 1_000_000 {
        return;
    }

    // `path` is the raw request target, query string included - the same
    // shape the H1 parser hands to `HttpRequest`.
    let target = match &data.query {
        Some(query) => format!("{}?{}", data.path, query),
        None => data.path.clone(),
    };

    let mut request = HttpRequest::new(data.method.as_str(), target);
    request.body = Bytes::from(data.body.clone());

    for (key, value) in &data.headers {
        if key.is_empty() {
            continue;
        }
        request.headers.insert(key.clone(), value.clone());
    }

    for (name, value) in &data.params {
        request.push_param(name, Bytes::from(value.clone()));
    }

    // Accessors - none of these may panic on arbitrary input.
    let _ = request.method_str();
    let _ = request.path.len();
    let _ = request.path_only();
    let _ = request.body.len();

    for (key, _) in request.headers.iter() {
        let _ = request.headers.get(key);
    }

    // Lazy query parsing over the fuzzer-controlled target.
    let view = request.query();
    let _ = view.len();
    for (key, value) in view.iter() {
        let _ = key.len();
        let _ = value.len();
        let _ = request.query_param(key);
    }

    for (name, _) in &data.params {
        let _ = request.param(name);
    }
});
