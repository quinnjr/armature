//! Authentication controller

use crate::models::{ApiResponse, AuthResponse, LoginRequest, RegisterRequest, UserResponse, ValidationError};
use crate::services::{get_auth_service, get_user_service};
use armature::armature_validation::Validate;
use armature::prelude::*;

#[controller("/api/auth")]
#[derive(Default, Clone)]
pub struct AuthController;

#[routes]
impl AuthController {
    /// POST /api/auth/login
    #[post("/login")]
    async fn login(req: HttpRequest) -> Result<HttpResponse, Error> {
        let body: LoginRequest = req.json().map_err(|e| {
            tracing::debug!(error = %e, "invalid login request body");
            Error::bad_request("Invalid request body")
        })?;

        if let Err(errors) = body.validate() {
            let details: Vec<ValidationError> = errors.into_iter().map(Into::into).collect();
            return HttpResponse::bad_request()
                .with_json(&ApiResponse::<()>::validation_error(details));
        }

        let auth_service = get_auth_service();

        let user = match get_user_service().find_by_email(&body.email) {
            Some(u) => u,
            None => {
                return HttpResponse::unauthorized().with_json(&ApiResponse::<()>::error(
                    "INVALID_CREDENTIALS",
                    "Invalid email or password",
                ));
            }
        };

        if !auth_service.verify_password(&body.password, &user.password_hash) {
            return HttpResponse::unauthorized().with_json(&ApiResponse::<()>::error(
                "INVALID_CREDENTIALS",
                "Invalid email or password",
            ));
        }

        let token = auth_service
            .generate_token(&user)
            .map_err(|_| Error::internal("Failed to generate token"))?;

        HttpResponse::ok().with_json(&ApiResponse::success(AuthResponse {
            token,
            token_type: "Bearer".to_string(),
            expires_in: auth_service.token_expiry_seconds(),
            user: UserResponse::from(&user),
        }))
    }

    /// POST /api/auth/register
    #[post("/register")]
    async fn register(req: HttpRequest) -> Result<HttpResponse, Error> {
        let body: RegisterRequest = req.json().map_err(|e| {
            tracing::debug!(error = %e, "invalid register request body");
            Error::bad_request("Invalid request body")
        })?;

        if let Err(errors) = body.validate() {
            let details: Vec<ValidationError> = errors.into_iter().map(Into::into).collect();
            return HttpResponse::bad_request()
                .with_json(&ApiResponse::<()>::validation_error(details));
        }

        let user_service = get_user_service();
        let auth_service = get_auth_service();

        if user_service.email_exists(&body.email) {
            return HttpResponse::conflict().with_json(&ApiResponse::<()>::error(
                "EMAIL_EXISTS",
                "Email already registered",
            ));
        }

        let password_hash = auth_service
            .hash_password(&body.password)
            .map_err(|_| Error::internal("Failed to hash password"))?;

        let user = user_service.create(body.email, password_hash, body.name);

        let token = auth_service
            .generate_token(&user)
            .map_err(|_| Error::internal("Failed to generate token"))?;

        HttpResponse::created().with_json(&ApiResponse::success(AuthResponse {
            token,
            token_type: "Bearer".to_string(),
            expires_in: auth_service.token_expiry_seconds(),
            user: UserResponse::from(&user),
        }))
    }
}
