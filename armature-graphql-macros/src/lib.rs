//! NestJS-style decorator macros for `armature-graphql` resolvers.
//!
//! `async-graphql`'s own `#[Object]` / `#[Subscription]` macros operate on a
//! whole `impl` block: every method in the block becomes a field on that one
//! GraphQL type. That means a single domain (e.g. "users") that needs a
//! query, a mutation, *and* a subscription has to be split into three
//! separate structs and three separate `impl` blocks by hand.
//!
//! This crate adds one macro, [`resolver`], that lets those three concerns
//! live together in a single `impl` block — the way a NestJS `@Resolver()`
//! class mixes `@Query()`, `@Mutation()`, and `@Subscription()` methods —
//! and splits them apart at compile time into the separate types
//! `async-graphql` needs, each driven by `async-graphql`'s real `#[Object]`
//! / `#[Subscription]` macros under the hood. Argument extraction, output
//! typing, descriptions, and error handling are therefore all
//! `async-graphql`'s own, unmodified behavior — this crate only rearranges
//! *which* methods go into *which* generated `impl` block.
//!
//! # Example
//!
//! ```ignore
//! use armature_graphql_macros::resolver;
//!
//! #[derive(Clone, Default)]
//! struct UserResolver;
//!
//! #[resolver]
//! impl UserResolver {
//!     #[query]
//!     async fn user(&self, id: async_graphql::ID) -> async_graphql::Result<User> {
//!         // ...
//!     }
//!
//!     #[mutation]
//!     async fn create_user(&self, input: CreateUserInput) -> async_graphql::Result<User> {
//!         // ...
//!     }
//!
//!     #[subscription]
//!     async fn user_created(&self) -> impl async_graphql::futures_util::Stream<Item = User> {
//!         // ...
//!     }
//! }
//! ```
//!
//! expands (roughly) to three wrapper types — `UserResolverQuery`,
//! `UserResolverMutation`, `UserResolverSubscription` — each a
//! `#[derive(Clone)] struct Wrapper(pub UserResolver);` that `Deref`s to
//! `UserResolver` (so fields on the original type are still reachable from
//! `self` inside the moved methods) and carries the matching real
//! `async-graphql` macro. Compose them into your schema roots with
//! [`async_graphql::MergedObject`]/[`async_graphql::MergedSubscription`]
//! alongside other resolvers, exactly as you would with hand-written
//! `#[Object]`/`#[Subscription]` types.
//!
//! Untagged methods in the `impl` block (plain helpers, no `#[query]`
//! /`#[mutation]`/`#[subscription]`) are left in place on the original type
//! and are not exposed as GraphQL fields.

mod resolver;

use proc_macro::TokenStream;

/// Split an `impl` block's `#[query]`/`#[mutation]`/`#[subscription]`-tagged
/// methods into the separate `async-graphql`-driven types a
/// `Schema<Query, Mutation, Subscription>` needs. See the [crate-level
/// docs](crate) for the full example and generated shape.
#[proc_macro_attribute]
pub fn resolver(attr: TokenStream, item: TokenStream) -> TokenStream {
    resolver::resolver_impl(attr, item)
}
