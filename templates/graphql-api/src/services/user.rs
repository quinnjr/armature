//! User service

use armature::prelude::*;
use async_graphql::ID;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::schema::{CreateUserInput, User, UserRole};

/// User service for managing users
///
/// `#[injectable]` makes this resolvable from the DI container, but nothing in
/// this template resolves it that way: `build_schema` in `main.rs` constructs
/// it directly and the schema owns it for the process lifetime. The attribute
/// is here so you can move it into the container without rewriting the type.
#[injectable]
#[derive(Clone)]
pub struct UserService {
    users: Arc<RwLock<HashMap<String, User>>>,
}

impl UserService {
    pub fn new() -> Self {
        let mut users = HashMap::new();

        // Add some sample users
        let sample_users = vec![
            User {
                id: ID::from("1"),
                name: "Alice Johnson".to_string(),
                email: "alice@example.com".to_string(),
                role: UserRole::Admin,
                created_at: Utc::now(),
            },
            User {
                id: ID::from("2"),
                name: "Bob Smith".to_string(),
                email: "bob@example.com".to_string(),
                role: UserRole::User,
                created_at: Utc::now(),
            },
            User {
                id: ID::from("3"),
                name: "Charlie Brown".to_string(),
                email: "charlie@example.com".to_string(),
                role: UserRole::User,
                created_at: Utc::now(),
            },
        ];

        for user in sample_users {
            users.insert(user.id.to_string(), user);
        }

        Self {
            users: Arc::new(RwLock::new(users)),
        }
    }

    /// List all users
    pub fn list(&self) -> Vec<User> {
        self.users.read().unwrap().values().cloned().collect()
    }

    /// Find user by ID
    pub fn find_by_id(&self, id: &ID) -> Option<User> {
        self.users.read().unwrap().get(&id.to_string()).cloned()
    }

    /// Find user by email
    pub fn find_by_email(&self, email: &str) -> Option<User> {
        self.users
            .read()
            .unwrap()
            .values()
            .find(|u| u.email == email)
            .cloned()
    }

    /// Create a new user
    pub fn create(&self, input: CreateUserInput) -> User {
        let id = uuid::Uuid::new_v4().to_string();

        let user = User {
            id: ID::from(&id),
            name: input.name,
            email: input.email,
            role: input.role.unwrap_or_default(),
            created_at: Utc::now(),
        };

        self.users.write().unwrap().insert(id, user.clone());
        user
    }

    /// Update a user's name
    pub fn update_name(&self, id: &ID, name: String) -> Option<User> {
        let mut users = self.users.write().unwrap();
        let user = users.get_mut(&id.to_string())?;
        user.name = name;
        Some(user.clone())
    }

    /// Delete a user
    pub fn delete(&self, id: &ID) -> bool {
        self.users
            .write()
            .unwrap()
            .remove(&id.to_string())
            .is_some()
    }
}

impl Default for UserService {
    fn default() -> Self {
        Self::new()
    }
}
