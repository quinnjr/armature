//! Behavioral tests for the derive macros: `Model`, `ApiModel`, `Resource`.
//!
//! Against the old code `#[derive(Model)]` implemented none of the traits it
//! advertised, `#[derive(ApiModel)]` ignored `#[api(skip)]` (leaking secret
//! fields), and `#[derive(Resource)]` ignored `#[resource(...)]` and returned
//! the struct identifier as the table name. These tests pin the fixed
//! behavior.

use armature_macros_utils::{ApiModel, Model, Resource};
use serde::{Deserialize, Serialize};

// NOTE: the user is responsible for the serde derives; `Model` only provides
// `Debug` + `Clone` + `new()` (see the retracted Serialize/Deserialize claim).
// This struct deliberately does NOT derive Debug or Clone — proving they come
// from `#[derive(Model)]`.
#[derive(Model, Default, PartialEq)]
struct User {
    id: i64,
    name: String,
}

#[test]
fn model_provides_clone() {
    let u = User {
        id: 1,
        name: "Alice".into(),
    };
    let c = u.clone();
    assert!(u == c);
}

#[test]
fn model_provides_debug() {
    let u = User {
        id: 7,
        name: "Bob".into(),
    };
    let rendered = format!("{u:?}");
    assert!(rendered.contains("User"));
    assert!(rendered.contains("name"));
    assert!(rendered.contains("Bob"));
}

#[test]
fn model_new_returns_default() {
    let u = User::new();
    assert_eq!(u.id, 0);
    assert_eq!(u.name, "");
}

#[derive(ApiModel, Serialize, Deserialize, Default)]
struct Account {
    id: i64,
    name: String,
    #[api(skip)]
    password: String,
}

#[test]
fn api_model_excludes_skipped_field_from_json() {
    let a = Account {
        id: 1,
        name: "Alice".into(),
        password: "s3cret".into(),
    };
    let json = a.to_json().unwrap();
    assert!(
        json.get("password").is_none(),
        "skipped field leaked: {json}"
    );
    assert_eq!(json.get("id").unwrap(), 1);
    assert_eq!(json.get("name").unwrap(), "Alice");
}

#[test]
fn api_model_roundtrips_non_skipped_fields() {
    let value = serde_json::json!({ "id": 9, "name": "Zed", "password": "" });
    let a = Account::from_json(&value).unwrap();
    assert_eq!(a.id, 9);
    assert_eq!(a.name, "Zed");
}

// Regression coverage: against the old code, `to_json` removed skipped
// fields by `stringify!(field_ident)`, ignoring serde's naming attributes.
// A struct-level `#[serde(rename_all = "camelCase")]` renames
// `password_hash` to `"passwordHash"` on the wire, so the old removal (which
// looked for the literal key `"password_hash"`) silently missed it and the
// secret leaked.
#[derive(ApiModel, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct SecretUser {
    user_id: i64,
    first_name: String,
    #[api(skip)]
    password_hash: String,
}

#[test]
fn api_model_excludes_skipped_field_under_rename_all() {
    let u = SecretUser {
        user_id: 1,
        first_name: "Alice".into(),
        password_hash: "s3cret".into(),
    };
    let json = u.to_json().unwrap();
    assert!(
        json.get("passwordHash").is_none(),
        "skipped field leaked under #[serde(rename_all)]: {json}"
    );
    // The non-skipped fields are still present under their renamed keys.
    assert_eq!(json.get("userId").unwrap(), 1);
    assert_eq!(json.get("firstName").unwrap(), "Alice");
}

// Same leak, via a field-level `#[serde(rename = "..")]` instead of a
// container-level `rename_all`.
#[derive(ApiModel, Serialize, Deserialize, Default)]
struct SecretToken {
    id: i64,
    #[serde(rename = "apiSecret")]
    #[api(skip)]
    secret: String,
}

#[test]
fn api_model_excludes_skipped_field_with_field_level_rename() {
    let t = SecretToken {
        id: 1,
        secret: "shh".into(),
    };
    let json = t.to_json().unwrap();
    assert!(
        json.get("apiSecret").is_none(),
        "skipped field leaked under #[serde(rename)]: {json}"
    );
    assert_eq!(json.get("id").unwrap(), 1);
}

#[derive(Resource, Default)]
#[resource(table = "users")]
struct UserEntity {
    #[resource(primary_key)]
    #[allow(dead_code)]
    id: i64,
    #[allow(dead_code)]
    name: String,
}

#[test]
fn resource_uses_configured_table_and_primary_key() {
    assert_eq!(UserEntity::table_name(), "users");
    assert_eq!(UserEntity::primary_key(), "id");
}

#[derive(Resource, Default)]
struct ProductVariant {
    #[allow(dead_code)]
    id: i64,
}

#[test]
fn resource_falls_back_to_snake_case_table_name() {
    assert_eq!(ProductVariant::table_name(), "product_variant");
    // No field is marked primary_key -> fall back to "id".
    assert_eq!(ProductVariant::primary_key(), "id");
}
