use armature_graphql_macros::resolver;
use async_graphql::{futures_util::stream::Stream, SimpleObject};

#[derive(Clone, SimpleObject)]
struct User {
    id: String,
    name: String,
}

#[derive(Clone, Default)]
pub struct UserResolver;

#[resolver]
impl UserResolver {
    #[query]
    async fn user(&self, id: String) -> async_graphql::Result<User> {
        Ok(User {
            id,
            name: "ada".to_string(),
        })
    }

    #[mutation]
    async fn create_user(&self, name: String) -> async_graphql::Result<User> {
        Ok(User {
            id: "1".to_string(),
            name,
        })
    }

    #[subscription]
    async fn user_created(&self) -> impl Stream<Item = User> {
        async_graphql::futures_util::stream::once(async {
            User {
                id: "1".to_string(),
                name: "ada".to_string(),
            }
        })
    }
}

#[derive(async_graphql::MergedObject)]
struct RootQuery(UserResolverQuery);

#[derive(async_graphql::MergedObject)]
struct RootMutation(UserResolverMutation);

#[derive(async_graphql::MergedSubscription)]
struct RootSubscription(UserResolverSubscription);

fn main() {
    let resolver = UserResolver;
    let query = RootQuery(resolver.clone().into());
    let mutation = RootMutation(resolver.clone().into());
    let subscription = RootSubscription(resolver.into());

    let _schema = async_graphql::Schema::build(query, mutation, subscription).finish();
}
