use armature_core::exception_filter::ExceptionContext;
use armature_core::{Error, HttpResponse};
use armature_proc_macro::catch;

#[catch]
async fn handle_all(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
    let _ = (error, ctx);
    HttpResponse::internal_server_error()
}

#[catch(NotFound, RouteNotFound)]
async fn handle_not_found(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
    let _ = (error, ctx);
    HttpResponse::not_found()
}

#[catch(Validation, priority = 100, name = "custom")]
async fn handle_validation(error: &Error, ctx: &ExceptionContext) -> HttpResponse {
    let _ = (error, ctx);
    HttpResponse::new(422)
}

fn main() {
    let _ = handle_all();
    let _ = handle_not_found();
    let _ = handle_validation();
}
