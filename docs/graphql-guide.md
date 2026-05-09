# GraphQL Guide

This guide explains how to use GraphQL with the Armature framework, inspired by NestJS's @nestjs/graphql.

## Overview

Armature provides optional GraphQL support through the `armature-graphql` module, built on top of `async-graphql`. This integration enables you to build type-safe GraphQL APIs with full dependency injection support and programmatic schema generation.

## Features

✅ **Type-Safe Schema** - Compile-time verified GraphQL types
✅ **Programmatic Schema** - Build schemas programmatically like NestJS
✅ **Queries & Mutations** - Full CRUD operations support
✅ **Subscriptions** - Real-time GraphQL subscriptions
✅ **DI Integration** - Services injected into resolvers
✅ **Decorator-Style** - Rust procedural macros for clean syntax
✅ **GraphiQL/Playground** - Built-in query interface
✅ **Schema Introspection** - Automatic API documentation

## Installation

Add the GraphQL feature to your `Cargo.toml`:

```toml
[dependencies]
armature-framework = { version = "0.1", features = ["graphql"] }
armature-graphql = "0.1"
```

## Quick Start

### 1. Define Your Types

```rust
use armature_graphql::{SimpleObject, ID};

#[derive(SimpleObject)]
struct Book {
    id: ID,
    title: String,
    author: String,
    year: i32,
}
```

### 2. Create Query Root

```rust
use armature_graphql::Object;

struct QueryRoot {
    book_service: BookService,
}

#[Object]
impl QueryRoot {
    async fn books(&self) -> Vec<Book> {
        self.book_service.get_all_books()
    }

    async fn book(&self, id: ID) -> Option<Book> {
        self.book_service.get_book_by_id(&id)
    }
}
```

### 3. Create Mutation Root

```rust
struct MutationRoot {
    book_service: BookService,
}

#[Object]
impl MutationRoot {
    async fn create_book(&self, title: String, author: String) -> Book {
        self.book_service.create_book(title, author)
    }
}
```

### 4. Build Schema Programmatically (NestJS-style)

```rust
use armature_graphql::{ProgrammaticSchemaBuilder, EmptySubscription};

// Using the programmatic builder
let schema = ProgrammaticSchemaBuilder::new()
    .query(QueryRoot { book_service: book_service.clone() })
    .mutation(MutationRoot { book_service: book_service.clone() })
    .subscription(EmptySubscription)
    .add_service(book_service)  // Add to schema context
    .build();

// Or use the standard Schema::build
let schema = Schema::build(
    QueryRoot { book_service },
    MutationRoot { book_service },
    EmptySubscription
).finish();
```

### 5. Create GraphQL Endpoint

```rust
router.add_route(Route {
    method: HttpMethod::POST,
    path: "/graphql".to_string(),
    handler: Arc::new(move |req| {
        let schema = schema.clone();
        Box::pin(async move {
            // Handle GraphQL request
            let gql_req: GraphQLRequest = req.json()?;
            let request = async_graphql::Request::new(gql_req.query);
            let response = schema.execute(request).await;

            let json = serde_json::to_value(&response)?;
            HttpResponse::ok().with_json(&json)
        })
    }),
});
```

## Type System

### Simple Objects

For simple data types:

```rust
#[derive(SimpleObject)]
struct User {
    id: ID,
    name: String,
    email: String,
    age: i32,
}
```

### Complex Objects with Resolvers

For types with computed fields:

```rust
struct User {
    id: ID,
    name: String,
}

#[Object]
impl User {
    // Simple field
    async fn id(&self) -> &ID {
        &self.id
    }

    // Computed field
    async fn full_name(&self, format: Option<String>) -> String {
        match format.as_deref() {
            Some("upper") => self.name.to_uppercase(),
            Some("lower") => self.name.to_lowercase(),
            _ => self.name.clone(),
        }
    }

    // Field with service injection (via context)
    async fn posts(&self, ctx: &Context<'_>) -> Vec<Post> {
        let service = ctx.data::<PostService>().unwrap();
        service.get_user_posts(&self.id)
    }
}
```

### Enums

```rust
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
enum Role {
    Admin,
    User,
    Guest,
}
```

### Unions

```rust
#[derive(Union)]
enum SearchResult {
    User(User),
    Post(Post),
    Comment(Comment),
}
```

### Input Objects

For mutation arguments:

```rust
#[derive(InputObject)]
struct CreateUserInput {
    name: String,
    email: String,
    age: Option<i32>,
}

#[Object]
impl MutationRoot {
    async fn create_user(&self, input: CreateUserInput) -> User {
        // Create user from input
    }
}
```

## Queries

### Basic Query

```rust
#[Object]
impl QueryRoot {
    async fn hello(&self) -> &str {
        "Hello, World!"
    }

    async fn users(&self) -> Vec<User> {
        self.user_service.get_all()
    }
}
```

**GraphQL:**
```graphql
query {
    hello
    users {
        id
        name
    }
}
```

### Query with Arguments

```rust
#[Object]
impl QueryRoot {
    async fn user(&self, id: ID) -> Result<User> {
        self.user_service
            .get_by_id(&id)
            .ok_or("User not found".into())
    }

    async fn search_users(
        &self,
        query: String,
        limit: Option<i32>,
    ) -> Vec<User> {
        self.user_service.search(&query, limit.unwrap_or(10))
    }
}
```

**GraphQL:**
```graphql
query {
    user(id: "123") {
        id
        name
    }

    searchUsers(query: "john", limit: 5) {
        id
        name
    }
}
```

### Nested Queries

```rust
struct User {
    id: ID,
    name: String,
}

#[Object]
impl User {
    async fn id(&self) -> &ID { &self.id }
    async fn name(&self) -> &str { &self.name }

    async fn posts(&self, ctx: &Context<'_>) -> Vec<Post> {
        ctx.data::<PostService>()
            .unwrap()
            .get_user_posts(&self.id)
    }
}

struct Post {
    id: ID,
    title: String,
}

#[Object]
impl Post {
    async fn id(&self) -> &ID { &self.id }
    async fn title(&self) -> &str { &self.title }

    async fn author(&self, ctx: &Context<'_>) -> User {
        ctx.data::<UserService>()
            .unwrap()
            .get_by_id(&self.author_id)
    }
}
```

**GraphQL:**
```graphql
query {
    user(id: "123") {
        name
        posts {
            title
            author {
                name
            }
        }
    }
}
```

## Mutations

### Basic Mutations

```rust
#[Object]
impl MutationRoot {
    async fn create_user(&self, name: String, email: String) -> User {
        self.user_service.create(name, email)
    }

    async fn update_user(&self, id: ID, name: String) -> Result<User> {
        self.user_service
            .update(&id, name)
            .ok_or("User not found".into())
    }

    async fn delete_user(&self, id: ID) -> bool {
        self.user_service.delete(&id)
    }
}
```

**GraphQL:**
```graphql
mutation {
    createUser(name: "John", email: "john@example.com") {
        id
        name
    }

    updateUser(id: "123", name: "Jane") {
        id
        name
    }

    deleteUser(id: "123")
}
```

### Mutations with Input Objects

```rust
#[derive(InputObject)]
struct CreatePostInput {
    title: String,
    content: String,
    author_id: ID,
    tags: Vec<String>,
}

#[Object]
impl MutationRoot {
    async fn create_post(&self, input: CreatePostInput) -> Post {
        self.post_service.create(input)
    }
}
```

**GraphQL:**
```graphql
mutation {
    createPost(input: {
        title: "My Post"
        content: "Post content"
        authorId: "123"
        tags: ["rust", "graphql"]
    }) {
        id
        title
    }
}
```

## Subscriptions

### Real-time Updates

```rust
use armature_graphql::Subscription;
use futures_util::Stream;

struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    async fn books(&self) -> impl Stream<Item = Book> {
        // Return a stream of books
        async_stream::stream! {
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                yield Book {
                    id: ID::from("1"),
                    title: "New Book".to_string(),
                    author: "Author".to_string(),
                    year: 2024,
                };
            }
        }
    }
}
```

**GraphQL:**
```graphql
subscription {
    books {
        id
        title
    }
}
```

## Programmatic Schema Building (NestJS-style)

Armature provides a `ProgrammaticSchemaBuilder` for building GraphQL schemas programmatically, similar to NestJS's approach:

```rust
use armature_graphql::ProgrammaticSchemaBuilder;

// Create services
let user_service = UserService::default();
let post_service = PostService::default();

// Create resolvers with injected services
let query = QueryRoot {
    user_service: user_service.clone(),
    post_service: post_service.clone(),
};

let mutation = MutationRoot {
    user_service: user_service.clone(),
    post_service: post_service.clone(),
};

// Build schema programmatically
let schema = ProgrammaticSchemaBuilder::new()
    .query(query)
    .mutation(mutation)
    .subscription(EmptySubscription)
    .add_service(user_service)    // Add to context
    .add_service(post_service)    // Add to context
    .build();
```

### Comparison with NestJS

**NestJS (@nestjs/graphql):**
```typescript
@Module({
  imports: [
    GraphQLModule.forRoot({
      autoSchemaFile: true,
    }),
  ],
  providers: [UserService, UserResolver],
})
export class AppModule {}

@Resolver(() => User)
export class UserResolver {
  constructor(private userService: UserService) {}

  @Query(() => [User])
  users() {
    return this.userService.findAll();
  }
}
```

**Armature (armature-graphql):**
```rust
#[injectable]
#[derive(Clone)]
struct UserService { }

struct QueryRoot {
    user_service: UserService,
}

#[Object]
impl QueryRoot {
    async fn users(&self) -> Vec<User> {
        self.user_service.find_all()
    }
}

let schema = ProgrammaticSchemaBuilder::new()
    .query(QueryRoot { user_service })
    .mutation(EmptyMutation)
    .subscription(EmptySubscription)
    .build();
```

## Dependency Injection with GraphQL

### Inject Services into Resolvers

```rust
// Define your service
#[injectable]
#[derive(Clone)]
struct UserService {
    database: DatabaseService,
}

// Method 1: Constructor injection (NestJS-style)
struct QueryRoot {
    user_service: UserService,
}

#[Object]
impl QueryRoot {
    async fn users(&self) -> Vec<User> {
        self.user_service.get_all()
    }
}

// Method 2: Context injection
// Add service to GraphQL context
let schema = ProgrammaticSchemaBuilder::new()
    .query(query)
    .add_service(user_service)  // Available in context
    .build();

// Use in resolver
#[Object]
impl QueryRoot {
    async fn users(&self, ctx: &Context<'_>) -> Vec<User> {
        let service = ctx.data::<UserService>().unwrap();
        service.get_all()
    }
}
```

## Error Handling

### Custom Errors

```rust
use armature_graphql::Error;

#[Object]
impl QueryRoot {
    async fn user(&self, id: ID) -> Result<User> {
        self.user_service
            .get_by_id(&id)
            .ok_or_else(|| Error::new("User not found"))
    }

    async fn validate_user(&self, email: String) -> Result<bool> {
        if !email.contains('@') {
            return Err(Error::new("Invalid email format"));
        }
        Ok(true)
    }
}
```

### Field Errors

```rust
#[Object]
impl User {
    async fn sensitive_data(&self, ctx: &Context<'_>) -> Result<String> {
        let auth = ctx.data::<AuthService>().unwrap();

        if !auth.is_authorized(&self.id) {
            return Err(Error::new("Unauthorized"));
        }

        Ok(self.sensitive_data.clone())
    }
}
```

## GraphQL Playground

### Built-in Playground

Armature provides two playground options:

#### GraphiQL (Lightweight)

```rust
use armature_graphql::graphiql_html;

router.add_route(Route {
    method: HttpMethod::GET,
    path: "/playground".to_string(),
    handler: Arc::new(move |_req| {
        Box::pin(async move {
            let html = graphiql_html("/graphql");
            Ok(HttpResponse::ok()
                .with_header("Content-Type".into(), "text/html".into())
                .with_body(html.into_bytes()))
        })
    }),
});
```

#### GraphQL Playground

```rust
use armature_graphql::graphql_playground_html;

let html = graphql_playground_html("/graphql");
```

## Best Practices

### 1. Use Input Objects for Complex Mutations

**Good:**
```rust
#[derive(InputObject)]
struct CreateUserInput {
    name: String,
    email: String,
    role: Role,
}

async fn create_user(&self, input: CreateUserInput) -> User
```

**Avoid:**
```rust
async fn create_user(&self, name: String, email: String, role: Role) -> User
```

### 2. Implement Pagination

```rust
#[derive(SimpleObject)]
struct UserConnection {
    edges: Vec<UserEdge>,
    page_info: PageInfo,
}

#[derive(SimpleObject)]
struct UserEdge {
    node: User,
    cursor: String,
}

#[derive(SimpleObject)]
struct PageInfo {
    has_next_page: bool,
    has_previous_page: bool,
}

#[Object]
impl QueryRoot {
    async fn users(&self, first: i32, after: Option<String>) -> UserConnection {
        self.user_service.paginate(first, after)
    }
}
```

### 3. Use DataLoader for N+1 Queries

```rust
use async_graphql::dataloader::*;

struct UserLoader {
    user_service: UserService,
}

#[async_trait::async_trait]
impl Loader<ID> for UserLoader {
    type Value = User;
    type Error = Arc<Error>;

    async fn load(&self, keys: &[ID]) -> Result<HashMap<ID, User>, Self::Error> {
        Ok(self.user_service.get_by_ids(keys))
    }
}
```

### 4. Add Field Descriptions

```rust
#[Object]
impl QueryRoot {
    /// Get all users in the system
    #[graphql(desc = "Retrieve a list of all users")]
    async fn users(&self) -> Vec<User> {
        self.user_service.get_all()
    }
}
```

### 5. Use Guards for Authorization

```rust
use async_graphql::Guard;

struct RoleGuard {
    role: Role,
}

#[async_trait::async_trait]
impl Guard for RoleGuard {
    async fn check(&self, ctx: &Context<'_>) -> Result<()> {
        let user = ctx.data::<CurrentUser>()?;
        if user.role == self.role {
            Ok(())
        } else {
            Err("Unauthorized".into())
        }
    }
}

#[Object]
impl MutationRoot {
    #[graphql(guard = "RoleGuard { role: Role::Admin }")]
    async fn delete_user(&self, id: ID) -> bool {
        self.user_service.delete(&id)
    }
}
```

## Testing

### Unit Testing Resolvers

```rust
#[tokio::test]
async fn test_query_users() {
    let service = UserService::default();
    let query = QueryRoot { user_service: service };

    let users = query.users().await;
    assert!(!users.is_empty());
}
```

### Integration Testing Schema

```rust
#[tokio::test]
async fn test_graphql_query() {
    let schema = create_schema();

    let query = r#"
        query {
            users {
                id
                name
            }
        }
    "#;

    let req = async_graphql::Request::new(query);
    let res = schema.execute(req).await;

    assert!(res.errors.is_empty());
    assert!(res.data.is_object());
}
```

## Performance Tips

1. **Use DataLoader** - Batch database queries
2. **Limit Query Depth** - Prevent deeply nested queries
3. **Add Query Complexity** - Limit computational cost
4. **Cache Results** - Use Redis or in-memory cache
5. **Optimize N+1** - Use DataLoader or JOIN queries

## Common Patterns

### Relay-Style Pagination

```rust
#[derive(SimpleObject)]
struct Connection<T> {
    edges: Vec<Edge<T>>,
    page_info: PageInfo,
}
```

### Error Union Pattern

```rust
#[derive(Union)]
enum UserResult {
    Success(User),
    Error(UserError),
}
```

### Batch Mutations

```rust
async fn batch_create_users(&self, inputs: Vec<CreateUserInput>) -> Vec<User>
```

## Summary

Armature's GraphQL support provides:

✅ **Type-Safe** - Compile-time schema validation
✅ **DI Integration** - Services injected into resolvers
✅ **Full Featured** - Queries, mutations, subscriptions
✅ **Developer Friendly** - Built-in playground
✅ **Production Ready** - Error handling, pagination, guards
✅ **Performant** - DataLoader, caching support

For more examples, see `examples/graphql_api.rs` in the repository.

