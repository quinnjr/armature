//! User controller

use crate::middleware::AuthMiddleware;
use crate::models::{ApiResponse, UserResponse};
use crate::services::get_user_service;
use armature::prelude::*;
use armature::{HandlerFn, MiddlewareChain};
use std::future::Future;
use std::sync::Arc;
use uuid::Uuid;

#[controller("/api/users")]
#[derive(Default, Clone)]
pub struct UserController;

#[routes]
impl UserController {
    /// GET /api/users - list users (auth required)
    #[get("")]
    async fn list(req: HttpRequest) -> Result<HttpResponse, Error> {
        with_auth(req, |_req| async move {
            let users: Vec<UserResponse> = get_user_service()
                .find_all()
                .iter()
                .map(UserResponse::from)
                .collect();

            HttpResponse::ok().with_json(&ApiResponse::success(users))
        })
        .await
    }

    /// GET /api/users/:id - get a user by id (auth required)
    #[get("/:id")]
    async fn get(req: HttpRequest) -> Result<HttpResponse, Error> {
        with_auth(req, |req| async move {
            let id = parse_user_id(&req)?;

            match get_user_service().find_by_id(id) {
                Some(user) => {
                    HttpResponse::ok().with_json(&ApiResponse::success(UserResponse::from(&user)))
                }
                None => HttpResponse::not_found()
                    .with_json(&ApiResponse::<()>::error("NOT_FOUND", "User not found")),
            }
        })
        .await
    }

    /// DELETE /api/users/:id - delete a user (auth required)
    #[delete("/:id")]
    async fn delete(req: HttpRequest) -> Result<HttpResponse, Error> {
        with_auth(req, |req| async move {
            let id = parse_user_id(&req)?;

            if get_user_service().delete(id) {
                Ok(HttpResponse::no_content())
            } else {
                HttpResponse::not_found()
                    .with_json(&ApiResponse::<()>::error("NOT_FOUND", "User not found"))
            }
        })
        .await
    }
}

fn parse_user_id(req: &HttpRequest) -> Result<Uuid, Error> {
    let id_str = req
        .param("id")
        .ok_or_else(|| Error::bad_request("User ID required"))?;

    Uuid::parse_str(id_str).map_err(|_| Error::bad_request("Invalid user ID format"))
}

/// Runs `handler` behind [`AuthMiddleware`], so every route that goes
/// through this requires a valid bearer token.
async fn with_auth<F, Fut>(req: HttpRequest, handler: F) -> Result<HttpResponse, Error>
where
    F: Fn(HttpRequest) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<HttpResponse, Error>> + Send + 'static,
{
    let mut chain = MiddlewareChain::new();
    chain.use_middleware(AuthMiddleware);

    let boxed: HandlerFn = Arc::new(move |req: HttpRequest| {
        Box::pin(handler(req))
            as std::pin::Pin<Box<dyn Future<Output = Result<HttpResponse, Error>> + Send>>
    });
    chain.apply(req, boxed).await
}
