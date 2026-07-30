//! Integration tests for the `#[resolver]` decorator macro: verifies that a
//! single `impl` block mixing `#[query]`/`#[mutation]`/`#[subscription]`
//! methods (NestJS-`@Resolver()`-style) actually executes through a real
//! `async_graphql::Schema`, end to end.

use armature_graphql::async_graphql::futures_util::{Stream, StreamExt};
use armature_graphql::async_graphql::{ID, Schema, SimpleObject};
use armature_graphql::resolver;

#[derive(SimpleObject, Clone)]
pub struct User {
    id: ID,
    name: String,
}

#[derive(Clone, Default)]
pub struct UserResolver {
    // Stands in for a DI-injected service the resolver depends on.
    greeting: String,
}

#[resolver]
impl UserResolver {
    #[query]
    async fn user(&self, id: ID) -> User {
        User {
            id,
            name: format!("{}user", self.greeting),
        }
    }

    #[mutation]
    async fn create_user(&self, name: String) -> User {
        User {
            id: ID::from("new"),
            name,
        }
    }

    #[subscription]
    async fn counter(&self) -> impl Stream<Item = i32> {
        armature_graphql::async_graphql::futures_util::stream::iter(0..3)
    }

    // Untagged helper methods stay on the original type and are not
    // exposed as GraphQL fields.
    fn helper(&self) -> &str {
        "not a field"
    }
}

#[derive(armature_graphql::async_graphql::MergedObject)]
struct Query(UserResolverQuery);

#[derive(armature_graphql::async_graphql::MergedObject)]
struct Mutation(UserResolverMutation);

type Sub = UserResolverSubscription;

type AppSchema = Schema<Query, Mutation, Sub>;

fn build_schema() -> AppSchema {
    let resolver = UserResolver {
        greeting: "hello-".to_string(),
    };
    Schema::build(
        Query(UserResolverQuery::from(resolver.clone())),
        Mutation(UserResolverMutation::from(resolver.clone())),
        UserResolverSubscription::from(resolver),
    )
    .finish()
}

#[tokio::test]
async fn query_field_executes_through_the_generated_wrapper() {
    let schema = build_schema();
    let res = schema.execute(r#"{ user(id: "1") { id name } }"#).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let data = serde_json::to_value(res.data).unwrap();
    assert_eq!(data["user"]["id"], "1");
    assert_eq!(data["user"]["name"], "hello-user");
}

#[tokio::test]
async fn mutation_field_executes_through_the_generated_wrapper() {
    let schema = build_schema();
    let res = schema
        .execute(r#"mutation { createUser(name: "Ada") { id name } }"#)
        .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let data = serde_json::to_value(res.data).unwrap();
    assert_eq!(data["createUser"]["name"], "Ada");
}

#[tokio::test]
async fn subscription_field_streams_through_the_generated_wrapper() {
    let schema = build_schema();
    let mut stream = schema.execute_stream(r#"subscription { counter }"#);

    for expected in 0..3 {
        let res = stream.next().await.unwrap();
        assert!(res.errors.is_empty(), "{:?}", res.errors);
        let data = serde_json::to_value(res.data).unwrap();
        assert_eq!(data["counter"], expected);
    }
    assert!(stream.next().await.is_none());
}

#[test]
fn untagged_methods_are_not_exposed_as_graphql_fields() {
    let resolver = UserResolver {
        greeting: "hi-".to_string(),
    };
    // Compiles only if `helper` stayed on the original type instead of
    // being moved into one of the generated wrapper types.
    assert_eq!(resolver.helper(), "not a field");
}
