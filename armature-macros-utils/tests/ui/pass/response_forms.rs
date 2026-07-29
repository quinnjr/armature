// Every documented form of the response macros must compile.
use armature_macros_utils::{html, json, redirect, text};
use serde_json::json as sjson;

fn main() {
    // json!: bare value, numeric status, `ok` alias.
    let _ = json!(sjson!({ "message": "Success", "id": 123 }));
    let _ = json!(200, sjson!({ "message": "Success" }));
    let _ = json!(ok, sjson!({ "user": "data" }));

    // html!: bare, numeric status, `ok` alias.
    let _ = html!("<html>...</html>");
    let _ = html!(200, "<h1>Hello</h1>");
    let _ = html!(ok, "<p>Content</p>");

    // text!: bare, numeric status, `ok` alias.
    let _ = text!("Hello, world!");
    let _ = text!(200, "Plain text content");
    let _ = text!(ok, "Success");

    // redirect!: bare (302), numeric status, keyword aliases.
    let _ = redirect!("/home");
    let _ = redirect!(301, "/new-location");
    let _ = redirect!(temporary, "/temp");
    let _ = redirect!(permanent, "/perm");
}
