use livekit::{
    publication::LocalTrackPublication,
    webrtc::{
        audio_source::native::NativeAudioSource,
        video_frame::{I420Buffer, VideoFrame, VideoRotation},
    },
};
use log::info;

use actix_web::web;
use parking_lot::Mutex;

use std::{fmt::Debug, sync::Arc};

use bevy::prelude::*;
use livekit_api::access_token::{AccessToken, VideoGrants};
use serde::{Deserialize, Serialize};

use crate::{LivekitRoom, LIVEKIT_API_KEY, LIVEKIT_API_SECRET, LIVEKIT_WS_URL};

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerMsg<T> {
    data: Option<T>,
    error: Option<String>,
}

impl<T: ToString> ServerMsg<T> {
    pub fn data(data: T) -> Self {
        Self { data: Some(data), error: None }
    }

    pub fn error(error: T) -> Self {
        let err_msg = error.to_string();
        log::warn!("Server error. {err_msg:?}");
        Self { data: None, error: Some(err_msg) }
    }
}

#[derive(Clone, Resource)]
pub struct ShutdownBevyRemotely {
    tx: crossbeam_channel::Sender<bool>,
    rx: crossbeam_channel::Receiver<bool>,
}

impl FromWorld for ShutdownBevyRemotely {
    fn from_world(_world: &mut World) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<bool>();
        Self { tx, rx }
    }
}

pub fn shutdown_bevy_remotely(
    mut app_exit_writer: EventWriter<bevy::app::AppExit>,
    shutdown: ResMut<ShutdownBevyRemotely>,
) {
    if let Ok(true) = shutdown.rx.try_recv() {
        log::info!("received bevy shutdown signal");
        app_exit_writer.send(bevy::app::AppExit);
    }
}

pub fn create_bot_token(room_name: String, ai_name: &str) -> anyhow::Result<String> {
    let api_key = std::env::var(LIVEKIT_API_KEY).unwrap();
    let api_secret = std::env::var(LIVEKIT_API_SECRET).unwrap();

    let ttl = std::time::Duration::from_secs(60 * 5); // 10 minutes (in sync with frontend)
    Ok(AccessToken::with_api_key(api_key.as_str(), api_secret.as_str())
        .with_ttl(ttl)
        .with_identity(ai_name)
        .with_name(ai_name)
        .with_grants(VideoGrants {
            room: room_name,
            room_list: true,
            room_join: true,
            room_admin: true,
            can_publish: true,
            room_record: true,
            can_subscribe: true,
            can_publish_data: true,
            can_update_own_metadata: true,
            ..Default::default()
        })
        .to_jwt()?)
}

pub struct RoomData {
    pub livekit_room: LivekitRoom,
    pub video_pub: LocalTrackPublication,
    pub stream_frame_data: crate::StreamingFrameData,
    pub audio_src: NativeAudioSource,
    pub audio_pub: LocalTrackPublication,
    pub bot_name: String,
}

pub async fn setup_and_connect_to_livekit(
    participant_room_name: String,
    video_frame_dimension: (u32, u32),
) -> anyhow::Result<RoomData> {
    let lvkt_url = std::env::var(LIVEKIT_WS_URL).unwrap();

    let bot_name = "kitt";

    // connect to webrtc room
    let lvkt_token = create_bot_token(participant_room_name, bot_name)?;

    let room_options = livekit::RoomOptions { ..Default::default() };

    let (room, room_events) = livekit::Room::connect(&lvkt_url, &lvkt_token, room_options).await?;
    let room = std::sync::Arc::new(room);

    info!("Established connection with livekit room. ID -> [{}]", room.name());

    // ************** SETUP OPENAI, TTS, & STT **************
    let crate::TracksPublicationData { video_pub, video_src, audio_src, audio_pub } =
        crate::publish_tracks(room.clone(), bot_name, video_frame_dimension).await?;

    let pixel_size = 4_u32;

    let (w, h) = (video_frame_dimension.0 as usize, video_frame_dimension.1 as usize);

    let frame_data = crate::FrameData {
        video_frame: Arc::new(Mutex::new(VideoFrame {
            rotation: VideoRotation::VideoRotation0,
            buffer: I420Buffer::new(w as u32, h as u32),
            timestamp_us: 0,
        })),
    };

    let stream_frame_data = crate::StreamingFrameData { pixel_size, video_src, frame_data };

    let livekit_room = LivekitRoom { room, room_events };

    Ok(RoomData {
        livekit_room,
        stream_frame_data,
        video_pub,
        audio_src,
        audio_pub,
        bot_name: bot_name.to_string(),
    })
}

mod health_check {
    pub async fn handler() -> impl actix_web::Responder {
        actix_web::HttpResponse::Ok().json(super::ServerMsg::data("OK"))
    }
}

pub type ServerStateMutex = parking_lot::Mutex<ServerResources>;

mod lsdk_webhook {
    use actix_web::{http::Method, web, HttpRequest, HttpResponse as Resp, Responder};

    use super::ServerMsg;

    use livekit_api::{
        access_token::{self},
        webhooks,
    };
    use log::info;

    pub async fn handler(
        req: HttpRequest,
        server_data: web::Data<super::ServerStateMutex>,
        body: web::Bytes,
    ) -> impl Responder {
        if req.method().ne(&Method::POST) {
            return Resp::MethodNotAllowed()
                .json(ServerMsg::error("Method not allowed".to_string()));
        }

        log::info!("SERVER RECEIVED WEBHOOK");

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

        let jwt = jwt.trim();

        let body = match std::str::from_utf8(&body) {
            Ok(i) => i,
            Err(e) => return Resp::BadRequest().json(ServerMsg::error(e.to_string())),
        };

        let event = match webhook_receiver.receive(body, jwt) {
            Ok(i) => i,
            Err(e) => return Resp::InternalServerError().json(ServerMsg::error(e.to_string())),
        };

        // room_finished
        if event.room.is_some() {
            let livekit_protocol::Room {
                name: participant_room_name,
                max_participants,
                num_participants,
                ..
            } = event.room.unwrap();
            let event = event.event;
            if event == "room_started" {
                if num_participants < max_participants {
                    info!("...connecting to room");

                    let server_data = server_data.lock();

                    log::info!("app state {:#?}", *server_data.app_state);

                    *server_data.app_state.lock() =
                        crate::ParticipantRoomName(participant_room_name);

                    log::info!("app state {:?}", *server_data.app_state);

                    info!("\nSERVER FINISHED PROCESSING ROOM_STARTED WEBHOOK");
                };
            } else if event == "room_finished" {
                let server_data = server_data.lock();

                log::info!("app state {:#?}", *server_data.app_state);

                *server_data.app_state.lock() =
                    crate::ParticipantRoomName(format!("reset:{participant_room_name}"));

                log::info!("app state {:?}", *server_data.app_state);

                info!("\nSERVER FINISHED PROCESSING ROOM_FINISHED WEBHOOK");
            }
        } else {
            info!("received event {}", event.event);
        }

        Resp::Ok().json(ServerMsg::data("Livekit Webhook Successfully Processed"))
    }
}

fn top_level_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/").service(web::resource("").to(health_check::handler)))
        .service(web::resource("/webhooks/livekit").to(lsdk_webhook::handler));
}

pub struct ServerResources {
    pub app_state: std::sync::Arc<parking_lot::Mutex<crate::ParticipantRoomName>>,
}

pub async fn http_server(
    tx: crossbeam_channel::Sender<actix_web::dev::ServerHandle>,
    app_state: std::sync::Arc<parking_lot::Mutex<crate::ParticipantRoomName>>,
) -> std::io::Result<()> {
    // let _ =  setAppState;
    let port = std::env::var("PORT")
        .unwrap_or_else(|_| "6669".to_string())
        .parse::<u16>()
        .expect("PORT couldn't be set");

    info!("starting HTTP server on port {port}");

    let server_resources =
        actix_web::web::Data::new(parking_lot::Mutex::new(ServerResources { app_state }));

    let server = actix_web::HttpServer::new(move || {
        actix_web::App::new()
            .wrap(actix_web::middleware::Compress::default())
            .wrap(actix_web::middleware::Logger::new("IP - %a | Time - %D ms"))
            .wrap(
                actix_web::middleware::DefaultHeaders::new()
                    .add(("Content-Type", "application/json")),
            )
            .app_data(server_resources.clone())
            .configure(top_level_routes)
    })
    .bind(("0.0.0.0", port))?
    .workers(1)
    .run();

    // server
    let _ = tx.send(server.handle());

    server.await
}

#[derive(Resource)]
pub struct ActixServer {
    server_handle: actix_web::dev::ServerHandle,
}

impl bevy::ecs::world::FromWorld for ActixServer {
    fn from_world(world: &mut World) -> Self {
        world.init_resource::<ShutdownBevyRemotely>();

        let app_state =
            std::sync::Arc::new(parking_lot::Mutex::new(crate::ParticipantRoomName::default()));

        world.insert_resource(crate::AppStateSync { state: app_state.clone(), dirty: false });

        let async_runtime = world.get_resource::<crate::AsyncRuntime>().unwrap();

        let (tx, rx) = crossbeam_channel::unbounded::<actix_web::dev::ServerHandle>();

        let shutdown_bev = world.get_resource::<ShutdownBevyRemotely>().unwrap();
        let shutdown_bev_tx = shutdown_bev.tx.clone();

        log::info!("spawning thread for server");

        let rt = async_runtime.rt.clone();

        std::thread::spawn(move || {
            let svr = http_server(tx, app_state);
            if let Err(e) = rt.block_on(svr) {
                log::info!("Server errored out | Reason {e:#?}");
            };
            log::warn!("Server exited  | Shutting down Bevy");
            shutdown_bev_tx.send(true).unwrap();
        });

        let server_handle = rx.recv().unwrap();

        Self { server_handle }
    }
}
