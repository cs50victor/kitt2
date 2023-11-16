use actix_web::{http::Method, web, HttpRequest, HttpResponse as Resp, Responder};
use std::sync::Arc;

use crate::{
    response::{CommonResponses, ServerMsg},
    state::ServerStateMutex,
    utils,
    webrtc::TurboLivekitConnector,
};
use livekit as lsdk;
use livekit_api::{
    access_token::{self},
    webhooks,
};
use log::info;

const BOT_NAME: &str = "talking_donut";

pub async fn handler(
    req: HttpRequest,
    server_data: web::Data<ServerStateMutex>,
    body: web::Bytes,
) -> impl Responder {
    if req.method().ne(&Method::POST) {
        return Resp::MethodNotAllowed().json(CommonResponses::MethodNotAllowed.json());
    }
    let token_verifier = match access_token::TokenVerifier::new() {
        Ok(i) => i,
        Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
    };
    let webhook_receiver = webhooks::WebhookReceiver::new(token_verifier);

    let jwt = req
        .headers()
        .get("Authorization")
        .and_then(|hv| hv.to_str().ok())
        .unwrap_or_default()
        .to_string();

    let body = match std::str::from_utf8(&body) {
        Ok(i) => i,
        Err(e) => return Resp::BadRequest().json(ServerMsg::error(e.to_string())),
    };

    let event = match webhook_receiver.receive(body, &jwt) {
        Ok(i) => i,
        Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
    };

    if event.room.is_some() && event.event == "room_started" {
        info!("ROOM STARTED 🎉");
        let livekit_protocol::Room {
            name: room_name,
            max_participants,
            num_participants,
            ..
        } = event.room.unwrap();
        if num_participants < max_participants {
            let lvkt_url = std::env::var("LIVEKIT_WS_URL").expect("LIVEKIT_WS_URL is not set");
            let lvkt_token = match utils::create_bot_token(room_name, BOT_NAME) {
                Ok(i) => i,
                Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
            };

            let (room, room_events) = match lsdk::Room::connect(
                &lvkt_url,
                &lvkt_token,
                lsdk::RoomOptions {
                    ..Default::default()
                },
            )
            .await
            {
                Ok(i) => i,
                Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
            };

            let room = Arc::new(room);

            info!("Established connection with room. ID -> [{}]", room.name());

            let mut turbo_webrtc = match TurboLivekitConnector::new(room, room_events).await {
                Ok(turbo_webrtc) => turbo_webrtc,
                Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
            };

            let mut server_data = server_data.lock();
            server_data.turbo_input_tx = Some(turbo_webrtc.get_txt_input_sender());
            server_data.turbo_livekit_connector_handle = Some(turbo_webrtc);

            info!("\nSERVER FINISHED PROCESSING ROOM_STARTED WEBHOOK");
        };
    } else {
        info!("received event {}", event.event);
    }

    Resp::Ok().json(ServerMsg::data("Livekit Webhook Successfully Processed"))
}

//  images will be in base64
// stt & images & text go in -> [find a way of batching all this information and sending it to GPT ] -> stream the response from OPENAI to livekit

// IT SHOULD NEVER TEXT & WRITE AT the same time
