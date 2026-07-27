use armature_core::guard::{Guard, GuardContext};
use armature_core::{Error, HttpRequest, HttpResponse};
use armature_proc_macro::{guard, use_guard};

#[derive(Default)]
struct AuthGuard;

#[async_trait::async_trait]
impl Guard for AuthGuard {
    async fn can_activate(&self, _ctx: &GuardContext) -> Result<bool, Error> {
        Ok(true)
    }
}

#[use_guard(AuthGuard)]
async fn protected(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

#[guard(AuthGuard::default())]
async fn protected_instance(req: HttpRequest) -> Result<HttpResponse, Error> {
    let _ = &req;
    Ok(HttpResponse::ok())
}

fn main() {}
