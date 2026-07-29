use armature_core::middleware::{Middleware, Next};
use armature_core::{Error, HttpRequest, HttpResponse};
use armature_proc_macro::{middleware, use_middleware};

struct NoopMiddleware;

#[async_trait::async_trait]
impl Middleware for NoopMiddleware {
    async fn handle(&self, req: HttpRequest, next: Next) -> Result<HttpResponse, Error> {
        next(req).await
    }
}

#[use_middleware(NoopMiddleware)]
async fn get_users(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[middleware(NoopMiddleware)]
async fn list_items(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

fn main() {}
