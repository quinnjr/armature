#![allow(dead_code)]
// Testing Example - Demonstrates testing utilities

use armature::prelude::*;
use serde::{Deserialize, Serialize};

// Example service to test
#[injectable]
#[derive(Clone)]
struct UserService {
    users: std::sync::Arc<std::sync::Mutex<Vec<User>>>,
}

impl Default for UserService {
    fn default() -> Self {
        Self {
            users: std::sync::Arc::new(std::sync::Mutex::new(vec![
                User {
                    id: 1,
                    name: "Alice".to_string(),
                    email: "alice@example.com".to_string(),
                },
                User {
                    id: 2,
                    name: "Bob".to_string(),
                    email: "bob@example.com".to_string(),
                },
            ])),
        }
    }
}

impl UserService {
    fn get_all(&self) -> Vec<User> {
        self.users.lock().unwrap().clone()
    }

    fn get_by_id(&self, id: i32) -> Option<User> {
        self.users
            .lock()
            .unwrap()
            .iter()
            .find(|u| u.id == id)
            .cloned()
    }

    fn create(&self, user: User) -> User {
        let mut users = self.users.lock().unwrap();
        users.push(user.clone());
        user
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct User {
    id: i32,
    name: String,
    email: String,
}

// Example controller to test
#[controller("/users")]
#[derive(Clone)]
struct UserController {
    user_service: UserService,
}

#[routes]
impl UserController {
    fn get_all(&self) -> Result<HttpResponse, Error> {
        HttpResponse::json(&self.user_service.get_all())
    }

    fn get_one(&self, id: i32) -> Result<HttpResponse, Error> {
        let user = self
            .user_service
            .get_by_id(id)
            .ok_or_else(|| Error::NotFound(format!("User {} not found", id)))?;
        HttpResponse::json(&user)
    }

    fn create(&self, user: User) -> Result<HttpResponse, Error> {
        HttpResponse::json(&self.user_service.create(user))
    }
}

#[tokio::main]
async fn main() {
    println!("🧪 Armature Testing Example");
    println!("===========================\n");

    println!("This example demonstrates how to test Armature applications.\n");
    println!("The testing module provides:");
    println!("  ✓ TestAppBuilder - Build test applications");
    println!("  ✓ TestClient - Make HTTP requests");
    println!("  ✓ TestResponse - Inspect responses");
    println!("  ✓ MockService - Create mock services");
    println!("  ✓ Assertions - Fluent assertions for responses");
    println!();
    println!("Run tests with: cargo test --features testing");
    println!();
    println!("See the test module below for examples.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use armature::armature_testing::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_get_all_users() {
        // Arrange
        let user_service = UserService::default();
        let controller = UserController {
            user_service: user_service.clone(),
        };

        let mut router = Router::new();
        let ctrl = controller.clone();
        router.add_route(Route {
            method: HttpMethod::GET,
            path: "/users".to_string(),
            handler: from_legacy_handler(Arc::new(move |_req: HttpRequest| {
                let c = ctrl.clone();
                Box::pin(async move { c.get_all()?.into_response() })
            })),
            constraints: None,
        });

        let client = TestClient::new(Arc::new(router));

        // Act
        let response = client.get("/users").await;

        // Assert
        assert_status(&response, 200);
        let users: Vec<User> = response.body_json().expect("Failed to parse JSON");
        assert_eq!(users.len(), 2);
        assert_eq!(users[0].name, "Alice");
    }

    #[tokio::test]
    async fn test_get_user_by_id() {
        // Arrange
        let user_service = UserService::default();
        let controller = UserController {
            user_service: user_service.clone(),
        };

        let mut router = Router::new();
        let ctrl = controller.clone();
        router.add_route(Route {
            method: HttpMethod::GET,
            path: "/users/:id".to_string(),
            handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
                let c = ctrl.clone();
                Box::pin(async move {
                    let id = req.param("id").and_then(|s| s.parse().ok()).unwrap_or(0);
                    c.get_one(id)?.into_response()
                })
            })),
            constraints: None,
        });

        let client = TestClient::new(Arc::new(router));

        // Act
        let response = client.get("/users/1").await;

        // Assert
        assert_status(&response, 200);
        let user: User = response.body_json().expect("Failed to parse JSON");
        assert_eq!(user.id, 1);
        assert_eq!(user.name, "Alice");
    }

    #[tokio::test]
    async fn test_user_not_found() {
        // Arrange
        let user_service = UserService::default();
        let controller = UserController {
            user_service: user_service.clone(),
        };

        let mut router = Router::new();
        let ctrl = controller.clone();
        router.add_route(Route {
            method: HttpMethod::GET,
            path: "/users/:id".to_string(),
            handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
                let c = ctrl.clone();
                Box::pin(async move {
                    let id = req.param("id").and_then(|s| s.parse().ok()).unwrap_or(0);
                    c.get_one(id)?.into_response()
                })
            })),
            constraints: None,
        });

        let client = TestClient::new(Arc::new(router));

        // Act
        let response = client.get("/users/999").await;

        // Assert - should be error
        response.assert_error();
    }

    #[test]
    fn test_mock_service() {
        // Create a mock service
        let mock = MockService::<String>::new().with_return("mocked value".to_string());

        // Record calls
        mock.record_call("method1");
        mock.record_call("method2");

        // Assert
        assert_eq!(mock.call_count(), 2);
        assert!(mock.was_called("method1"));
        assert!(mock.was_called("method2"));
        assert_eq!(mock.get_return(), Some("mocked value".to_string()));
    }

    #[test]
    fn test_spy() {
        // Create a spy wrapping a real service
        let service = UserService::default();
        let spy = Spy::new(service);

        // Use the service and record calls
        spy.record("get_all");
        let _users = spy.inner().get_all();
        spy.record("get_all_completed");

        // Assert on spy
        assert_eq!(spy.call_count(), 2);
        assert!(spy.was_called("get_all"));
    }
}
