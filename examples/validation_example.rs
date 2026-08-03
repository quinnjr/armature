#![allow(
    dead_code,
    unused_imports,
    clippy::default_constructed_unit_structs,
    clippy::needless_borrow,
    clippy::unnecessary_lazy_evaluations
)]
// Validation Example - Demonstrates validation framework

use armature::armature_validation::*;
use armature::prelude::*;
use serde::{Deserialize, Serialize};

// ========== DTOs with Validation ==========

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CreateUserDto {
    pub name: String,
    pub email: String,
    pub age: i32,
    pub password: String,
}

impl Validate for CreateUserDto {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        // Validate name
        if let Err(e) = NotEmpty::validate(&self.name, "name") {
            errors.push(e);
        }
        if let Err(e) = MinLength(2).validate(&self.name, "name") {
            errors.push(e);
        }
        if let Err(e) = MaxLength(50).validate(&self.name, "name") {
            errors.push(e);
        }

        // Validate email
        if let Err(e) = NotEmpty::validate(&self.email, "email") {
            errors.push(e);
        }
        if let Err(e) = IsEmail::validate(&self.email, "email") {
            errors.push(e);
        }

        // Validate age
        if let Err(e) = IsPositive::validate_i32(self.age, "age") {
            errors.push(e);
        }
        let age_range = InRange { min: 18, max: 120 };
        if let Err(e) = age_range.validate(self.age, "age") {
            errors.push(e);
        }

        // Validate password
        if let Err(e) = MinLength(8).validate(&self.password, "password") {
            errors.push(e);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UpdateUserDto {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl Validate for UpdateUserDto {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        if let Some(ref name) = self.name
            && let Err(e) = MinLength(2).validate(name, "name")
        {
            errors.push(e);
        }

        if let Some(ref email) = self.email
            && let Err(e) = IsEmail::validate(email, "email")
        {
            errors.push(e);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
    pub age: i32,
}

// ========== Services ==========

#[injectable]
#[derive(Clone, Default)]
struct UserService;

impl UserService {
    fn create(&self, dto: CreateUserDto) -> Result<User, Error> {
        Ok(User {
            id: 1,
            name: dto.name,
            email: dto.email,
            age: dto.age,
        })
    }

    fn update(&self, id: i32, dto: UpdateUserDto) -> Result<User, Error> {
        Ok(User {
            id,
            name: dto.name.unwrap_or_else(|| "Updated".to_string()),
            email: dto
                .email
                .unwrap_or_else(|| "updated@example.com".to_string()),
            age: 30,
        })
    }
}

// ========== Controllers ==========

#[controller("/users")]
#[derive(Clone, Default)]
struct UserController;

#[routes]
impl UserController {
    #[post("")]
    async fn create(req: HttpRequest) -> Result<HttpResponse, Error> {
        let dto: CreateUserDto = ValidationPipe::parse(&req)?;
        let service = UserService::default();
        let user = service.create(dto)?;
        HttpResponse::json(&user)
    }

    #[put("/:id")]
    async fn update(req: HttpRequest) -> Result<HttpResponse, Error> {
        let id = req.param("id").and_then(|s| s.parse().ok()).unwrap_or(0);

        let dto: UpdateUserDto = ValidationPipe::parse(&req)?;
        let service = UserService::default();
        let user = service.update(id, dto)?;
        HttpResponse::json(&user)
    }
}

// ========== Module ==========

#[module(
    providers: [UserService],
    controllers: [UserController]
)]
#[derive(Default, Clone)]
struct AppModule;

#[tokio::main]
async fn main() {
    println!("✅ Armature Validation Example");
    println!("==============================\n");

    println!("Server running on http://localhost:3018");
    println!();
    println!("Validation Features:");
    println!("  ✓ String validators (NotEmpty, MinLength, MaxLength)");
    println!("  ✓ Format validators (IsEmail, IsUrl, IsUuid)");
    println!("  ✓ Pattern validators (IsAlpha, IsAlphanumeric, IsNumeric)");
    println!("  ✓ Number validators (Min, Max, IsPositive, InRange)");
    println!("  ✓ Custom validators (Matches)");
    println!("  ✓ Validation pipe for automatic validation");
    println!("  ✓ Detailed error messages");
    println!();
    println!("Endpoints:");
    println!("  POST /users - Create user (with validation)");
    println!("  PUT /users/:id - Update user (with validation)");
    println!();
    println!("Example requests:");
    println!();
    println!("Valid:");
    println!("  curl -X POST http://localhost:3018/users \\");
    println!("    -H 'Content-Type: application/json' \\");
    println!(
        "    -d '{{\"name\":\"John\",\"email\":\"john@example.com\",\"age\":30,\"password\":\"secure123\"}}'"
    );
    println!();
    println!("Invalid (will return validation errors):");
    println!("  curl -X POST http://localhost:3018/users \\");
    println!("    -H 'Content-Type: application/json' \\");
    println!("    -d '{{\"name\":\"\",\"email\":\"invalid\",\"age\":10,\"password\":\"short\"}}'");
    println!();

    let app = Application::create::<AppModule>().await;

    if let Err(e) = app.listen(3018).await {
        eprintln!("Server error: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_user() {
        let dto = CreateUserDto {
            name: "John Doe".to_string(),
            email: "john@example.com".to_string(),
            age: 30,
            password: "securepass123".to_string(),
        };

        assert!(dto.validate().is_ok());
    }

    #[test]
    fn test_invalid_email() {
        let dto = CreateUserDto {
            name: "John".to_string(),
            email: "invalid-email".to_string(),
            age: 30,
            password: "securepass123".to_string(),
        };

        let result = dto.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.field == "email"));
    }

    #[test]
    fn test_age_validation() {
        let dto = CreateUserDto {
            name: "John".to_string(),
            email: "john@example.com".to_string(),
            age: 10,
            password: "securepass123".to_string(),
        };

        let result = dto.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.field == "age"));
    }

    #[test]
    fn test_short_password() {
        let dto = CreateUserDto {
            name: "John".to_string(),
            email: "john@example.com".to_string(),
            age: 30,
            password: "short".to_string(),
        };

        let result = dto.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.iter().any(|e| e.field == "password"));
    }

    #[test]
    fn test_multiple_errors() {
        let dto = CreateUserDto {
            name: "".to_string(),
            email: "invalid".to_string(),
            age: -5,
            password: "x".to_string(),
        };

        let result = dto.validate();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.len() >= 4);
    }
}
