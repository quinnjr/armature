// GraphQL decorator-style macros and utilities

/// Re-export async-graphql macros for decorator-like syntax
pub use async_graphql::{
    ComplexObject, Enum, InputObject, Interface, Object, SimpleObject, Subscription, Union,
};

// Field decorators (using async-graphql attributes)
// These are documentation helpers to show NestJS-like patterns

/// Field decorator - marks a field as a GraphQL field
///
/// Usage in NestJS style:
/// ```ignore
/// use armature_graphql::decorators::*;
///
/// #[derive(SimpleObject)]
/// struct User {
///     #[graphql(name = "userId")]  // Custom field name
///     id: ID,
///
///     #[graphql(desc = "User's email address")]  // Field description
///     email: String,
///
///     #[graphql(skip)]  // Skip this field
///     password: String,
/// }
/// ```
pub mod field {
    pub use async_graphql::ID;
}

/// Resolver decorators
pub mod resolver {
    /// Marks a struct as a GraphQL resolver
    /// Use #[Object] for query/mutation resolvers
    /// Use #[Subscription] for subscription resolvers
    pub use async_graphql::{Object, Subscription};
}

/// Input type decorators
pub mod input {
    /// Marks a struct as a GraphQL input type
    pub use async_graphql::InputObject;
}

/// Type decorators
pub mod types {
    pub use async_graphql::{Enum, Interface, SimpleObject, Union};
}

/// Context decorators
pub mod context {
    /// Context injection in resolvers
    pub use async_graphql::Context;
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_graphql::ID;

    #[derive(SimpleObject)]
    struct TestUser {
        id: ID,
        name: String,
    }

    #[test]
    fn test_simple_object() {
        let user = TestUser {
            id: ID::from("1"),
            name: "Test".to_string(),
        };
        assert_eq!(user.name, "Test");
    }
}
