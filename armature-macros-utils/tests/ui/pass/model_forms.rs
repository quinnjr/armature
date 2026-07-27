// Every documented form of the derive macros must compile.
use armature_macros_utils::{ApiModel, Model, Resource};
use serde::{Deserialize, Serialize};

// `Model` provides Debug + Clone + new(); the user adds the serde derives.
#[derive(Model, Default)]
struct BasicModel {
    id: i64,
    name: String,
}

#[derive(ApiModel, Serialize, Deserialize, Default)]
struct ApiUser {
    id: i64,
    name: String,
    #[api(skip)]
    password_hash: String,
}

#[derive(Resource, Serialize, Deserialize, Default)]
#[resource(table = "users")]
struct UserEntity {
    #[resource(primary_key)]
    id: i64,
    name: String,
}

fn main() {
    let m = BasicModel::new();
    let cloned = m.clone();
    let _ = format!("{cloned:?}");

    let u = ApiUser {
        id: 1,
        name: "Alice".into(),
        password_hash: "x".into(),
    };
    let _ = u.to_json();

    let _ = UserEntity::table_name();
    let _ = UserEntity::primary_key();
}
