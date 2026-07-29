//! Behavioral tests for the `Query` derive.
//!
//! Regression: values containing `&`, `=` or `%` must round-trip. The old
//! implementation rebuilt a query string with `format!("{}={}", k, v)` over
//! already-decoded pairs and fed it to `serde_urlencoded::from_str` without
//! percent-encoding, corrupting any value that contained a delimiter.

use armature_core::HttpRequest;
use armature_proc_macro::Query;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Query, Deserialize, Debug, PartialEq)]
struct Filters {
    q: String,
    page: Option<u32>,
}

fn request_with_query(pairs: &[(&str, &str)]) -> HttpRequest {
    let mut query = HashMap::new();
    for (k, v) in pairs {
        query.insert(k.to_string(), v.to_string());
    }
    HttpRequest::from_parts(
        "GET".to_string(),
        "/search".to_string(),
        HashMap::new(),
        vec![],
        HashMap::new(),
        query,
    )
}

#[test]
fn value_with_ampersand_and_equals_round_trips() {
    let req = request_with_query(&[("q", "a&b=c")]);
    let filters = Filters::from_query(&req).expect("query must deserialize");
    assert_eq!(filters.q, "a&b=c");
    assert_eq!(filters.page, None);
}

#[test]
fn value_with_percent_round_trips() {
    let req = request_with_query(&[("q", "100%25 off")]);
    let filters = Filters::from_query(&req).expect("query must deserialize");
    assert_eq!(filters.q, "100%25 off");
}

#[test]
fn plain_values_still_work() {
    let req = request_with_query(&[("q", "hello"), ("page", "3")]);
    let filters = Filters::from_query(&req).expect("query must deserialize");
    assert_eq!(filters.q, "hello");
    assert_eq!(filters.page, Some(3));
}
