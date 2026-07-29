//! User service

use crate::models::{User, UserRole};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use uuid::Uuid;

pub struct UserService {
    users: RwLock<HashMap<Uuid, User>>,
}

impl UserService {
    pub fn new() -> Self {
        Self {
            users: RwLock::new(HashMap::new()),
        }
    }

    pub fn find_by_id(&self, id: Uuid) -> Option<User> {
        self.users.read().unwrap().get(&id).cloned()
    }

    pub fn find_by_email(&self, email: &str) -> Option<User> {
        self.users
            .read()
            .unwrap()
            .values()
            .find(|u| u.email == email)
            .cloned()
    }

    pub fn find_all(&self) -> Vec<User> {
        self.users.read().unwrap().values().cloned().collect()
    }

    pub fn create(&self, email: String, password_hash: String, name: String) -> User {
        self.create_with_role(email, password_hash, name, UserRole::User)
    }

    /// Create a user with an explicit role. Used by `create` (always
    /// `UserRole::User`) and by `main` to seed the default admin account.
    pub fn create_with_role(
        &self,
        email: String,
        password_hash: String,
        name: String,
        role: UserRole,
    ) -> User {
        let id = Uuid::new_v4();
        let now = Utc::now();

        let user = User {
            id,
            email,
            password_hash,
            name,
            role,
            created_at: now,
            updated_at: now,
        };

        self.users.write().unwrap().insert(id, user.clone());
        user
    }

    pub fn update(&self, id: Uuid, name: Option<String>) -> Option<User> {
        let mut users = self.users.write().unwrap();

        if let Some(user) = users.get_mut(&id) {
            if let Some(n) = name {
                user.name = n;
            }
            user.updated_at = Utc::now();
            return Some(user.clone());
        }

        None
    }

    pub fn delete(&self, id: Uuid) -> bool {
        self.users.write().unwrap().remove(&id).is_some()
    }

    pub fn count(&self) -> usize {
        self.users.read().unwrap().len()
    }

    pub fn email_exists(&self, email: &str) -> bool {
        self.find_by_email(email).is_some()
    }
}

impl Default for UserService {
    fn default() -> Self {
        Self::new()
    }
}

static USER_SERVICE: OnceLock<UserService> = OnceLock::new();

/// Install the process-wide [`UserService`].
///
/// Must be called exactly once from `main`, before the server starts
/// accepting requests.
pub fn init_user_service() {
    if USER_SERVICE.set(UserService::new()).is_err() {
        panic!("UserService already initialized");
    }
}

/// Access the process-wide [`UserService`].
pub fn get_user_service() -> &'static UserService {
    USER_SERVICE
        .get()
        .expect("UserService not initialized — call init_user_service() in main()")
}

