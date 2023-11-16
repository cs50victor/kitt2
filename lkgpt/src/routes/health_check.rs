use actix_web::{HttpResponse as Resp, Responder};

use crate::response::ServerMsg;

pub async fn handler() -> impl Responder {
    Resp::Ok().json(ServerMsg::data("OK"))
}
