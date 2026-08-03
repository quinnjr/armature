// Validation pipe for automatic request validation

use crate::{Validate, ValidationErrors};
use armature_core::{Error, HttpRequest, HttpResponse};
use serde::de::DeserializeOwned;

/// Validation pipe that validates request bodies
pub struct ValidationPipe;

impl ValidationPipe {
    /// Validate and parse request body
    pub fn parse<T>(req: &HttpRequest) -> Result<T, Error>
    where
        T: DeserializeOwned + Validate,
    {
        // Parse JSON
        let parsed: T = serde_json::from_slice(&req.body)
            .map_err(|e| Error::BadRequest(format!("Invalid JSON: {}", e)))?;

        // Validate
        parsed
            .validate()
            .map_err(|errors| Error::BadRequest(format!("Validation failed: {:?}", errors)))?;

        Ok(parsed)
    }

    /// Transform validation errors to HTTP response
    pub fn error_response(errors: ValidationErrors) -> HttpResponse {
        HttpResponse::from_parts(
            400,
            std::collections::HashMap::from([(
                "Content-Type".to_string(),
                "application/json".to_string(),
            )]),
            errors.to_json().to_string().into_bytes(),
        )
    }
}

/// Macro to validate a DTO in a handler
#[macro_export]
macro_rules! validate {
    ($dto:expr) => {{
        $dto.validate()
            .map_err(|errors| armature_core::Error::BadRequest(format!("{:?}", errors)))?
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Validate, ValidationError};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Serialize)]
    struct TestDto {
        name: String,
        age: i32,
    }

    impl Validate for TestDto {
        fn validate(&self) -> Result<(), Vec<ValidationError>> {
            let mut errors = Vec::new();

            if self.name.is_empty() {
                errors.push(ValidationError::new("name", "Name is required"));
            }

            if self.age < 0 {
                errors.push(ValidationError::new("age", "Age must be positive"));
            }

            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors)
            }
        }
    }

    #[test]
    fn test_validation_pipe() {
        let valid_dto = TestDto {
            name: "John".to_string(),
            age: 30,
        };

        let json = serde_json::to_vec(&valid_dto).unwrap();
        let mut req = HttpRequest::new("POST", "/test".to_string());
        req.body = armature_core::Bytes::from(json);

        let result: Result<TestDto, Error> = ValidationPipe::parse(&req);
        assert!(result.is_ok());
    }
}
