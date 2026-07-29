//! Authentication middleware for protected routes.

use crate::services::get_auth_service;
use armature::prelude::*;
use armature::{Middleware, Next};

/// Requires a valid `Authorization: Bearer <token>` header.
///
/// `UserController` wraps its handlers in this middleware (via a small
/// `with_auth` helper) so every route it protects requires a valid JWT.
#[derive(Default)]
pub struct AuthMiddleware;

#[async_trait::async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        let token = req
            .headers
            .get("authorization")
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(|s| s.to_string());

        match token {
            Some(t) => match get_auth_service().verify_token(&t) {
                Ok(_claims) => next(req).await,
                Err(e) => {
                    // Log the concrete failure (expired/invalid-signature/config
                    // error/etc.) server-side only — returning it to the client
                    // would let them distinguish failure modes and use this
                    // endpoint as a token-validation oracle.
                    tracing::warn!(error = %e, "JWT verification failed");
                    Err(Error::unauthorized("Invalid or expired token"))
                }
            },
            None => Err(Error::unauthorized("Missing authorization header")),
        }
    }
}
