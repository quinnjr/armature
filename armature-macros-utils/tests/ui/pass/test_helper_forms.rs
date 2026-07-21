// Every documented form of the test-helper macros must compile.
use armature_core::HttpResponse;
use armature_macros_utils::{assert_json, assert_status, test_request};

fn main() {
    // test_request!: GET, POST + body, GET + headers.
    let _req = test_request!(GET "/users");
    let _req = test_request!(POST "/users", serde_json::json!({ "name": "Alice" }));
    let _req = test_request!(GET "/users/123", headers: { "Authorization": "Bearer token" });

    let resp = HttpResponse::new(200)
        .with_json(&serde_json::json!({ "id": 1, "name": "Alice" }))
        .unwrap();

    // assert_status!: numeric and `ok` alias.
    assert_status!(resp, 200);
    assert_status!(resp, ok);

    // assert_json!: object-literal form.
    assert_json!(resp, { "id": 1, "name": "Alice" });
}
