use std::sync::Arc;

use actix_web::http::Method;
use actix_web::{web, HttpRequest, HttpResponse as Resp, Responder};

use livekit as lsdk;
use livekit::{
    options::{TrackPublishOptions, VideoCodec},
    track::{LocalTrack, TrackSource},
};
use livekit_api::{
    access_token::{self, AccessToken, VideoGrants},
    webhooks,
};
use log::{info, warn};

use lsdk::RoomEvent;
use lsdk::{
    track::LocalVideoTrack,
    webrtc::video_source::{native, RtcVideoSource, VideoResolution},
};
use tokio::sync::mpsc;

use crate::{
    response::{CommonResponses, ServerMsg},
    state::ServerStateMutex,
};

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
            let lvkt_token = match create_token(room_name, "talking donut") {
                Ok(i) => i,
                Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
            };

            let (room, rx) =
                match lsdk::Room::connect(&lvkt_url, &lvkt_token, lsdk::RoomOptions::default())
                    .await
                {
                    Ok(i) => i,
                    Err(e) => {
                        warn!("Error connecting to room - {}", e);
                        return Resp::InternalServerError().json(ServerMsg::error(e.to_string()))
                    }
                };
            let room = Arc::new(room);

            info!("Connected to room. ID -> [{}]", room.name());

            let room_msg_handler = tokio::spawn(async move {
                match handle_room_events(rx).await {
                    Ok(_) => info!("Room Event handler thread exited in a smooth fashion"),
                    Err(e) => info!("Room Event handler exited with error: {}", e),
                }
            });

            let (width, height) = (1920, 1080);
            let livekit_vid_src = native::NativeVideoSource::new(VideoResolution { width, height });
            let track = LocalVideoTrack::create_video_track(
                "her",
                RtcVideoSource::Native(livekit_vid_src.clone()),
            );

            let mut server_data = server_data.lock();

            let turbo_webrtc =
                tokio::spawn(engine_turbo::TurboWebrtcConnector::new(livekit_vid_src));

            server_data.turbo_webrtc_connector_handle = Some(turbo_webrtc);
            server_data.room_msg_thread_handler = Some(room_msg_handler);

            match room
                .local_participant()
                .publish_track(
                    LocalTrack::Video(track),
                    TrackPublishOptions {
                        source: TrackSource::Camera,
                        video_codec: VideoCodec::VP8,
                        ..Default::default()
                    },
                )
                .await
            {
                Ok(i) => i,
                Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
            };
        }
    } else {
        info!("received event {}", event.event);
    }

    Resp::Ok().json(ServerMsg::data("Livekit Webhook Successfully Processed"))
}

fn create_token(room_name: String, ai_name: &str) -> anyhow::Result<String> {
    let api_key = std::env::var("LIVEKIT_API_KEY")?;
    let api_secret = std::env::var("LIVEKIT_API_SECRET")?;

    let ttl = std::time::Duration::from_secs(60 * 5); // 5 minutes (in sync with frontend)
    Ok(
        AccessToken::with_api_key(api_key.as_str(), api_secret.as_str())
            .with_ttl(ttl)
            .with_identity(ai_name)
            .with_name(ai_name)
            .with_grants(VideoGrants {
                room_list: true,
                room_record: true,
                room_join: true,
                room: room_name,
                can_publish: true,
                can_subscribe: true,
                can_publish_data: true,
                can_update_own_metadata: true,
                ..Default::default()
            })
            .to_jwt()?,
    )
}

async fn handle_room_events(mut rx: mpsc::UnboundedReceiver<RoomEvent>) -> anyhow::Result<()> {
    loop {
        match rx.recv().await {
            None => {
                warn!("Disconnected");
                break;
                // match e {
                //     tokio::sync::mpsc::error::TryRecvError::Disconnected => {
                //     }
                //     tokio::sync::mpsc::error::TryRecvError::Empty => {
                //         // warn!("Empty");
                //     }
                // }
            }
            Some(room_event) => {
                match room_event {
                    RoomEvent::DataReceived {
                        payload,
                        kind: _,
                        participant: _,
                    } => {
                        println!("Received payload data {:?}", payload);
                        // if let DataPacketKind::Reliable = kind {
                        // }
                    }
                    _ => {
                        info!("msg {:?}", room_event);
                    }
                }
            }
        }
    }
    warn!("\n\n\nDONE");
    Ok(())
}
