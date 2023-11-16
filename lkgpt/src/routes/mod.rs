mod health_check;
mod lsdk_webhook;
use actix_web::web;

pub fn top_level_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/").service(web::resource("").to(health_check::handler)))
        .service(web::resource("/lsdk-webhook").to(lsdk_webhook::handler));
}
