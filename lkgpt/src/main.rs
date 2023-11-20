#![feature(ascii_char,async_closure)]
mod assets;
mod core;
mod gpt;
mod room_events;
mod scene;
mod stt;
mod track_pub;
mod tts;
mod turbo;
mod webrtc;

mod response;
mod routes;
mod state;
mod utils;

use actix_web::{middleware, web::Data, App, HttpServer};

use log::{error, info};
use std::{env, thread};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    dotenvy::dotenv().ok();

    std::env::var("LIVEKIT_API_KEY").expect("LIVEKIT_API_KEY must be set");
    std::env::var("LIVEKIT_API_SECRET").expect("LIVEKIT_API_SECRET must be set");
    let port = env::var("PORT")
        .unwrap_or_else(|_| "6669".to_string())
        .parse::<u16>()
        .expect("PORT couldn't be set");

    let mut formatted_builder = pretty_env_logger::formatted_builder();
    let pretty_env_builder = formatted_builder
        .filter_module("lkgpt", log::LevelFilter::Info)
        .filter_module("actix_server", log::LevelFilter::Info)
        .filter_module("actix_web", log::LevelFilter::Info);
    if cfg!(target_os = "unix"){
        pretty_env_builder.filter_module("livekit", log::LevelFilter::Info);
    }
    pretty_env_builder.init();

    let server_data = Data::new(parking_lot::Mutex::new(state::ServerState::new()));

    info!("starting HTTP server on port {port}");

    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::new("IP - %a | Time - %D ms"))
            .wrap(middleware::DefaultHeaders::new().add(("Content-Type", "application/json")))
            .app_data(server_data.clone())
            .configure(routes::top_level_routes)
    })
    .bind(("0.0.0.0", port))?
    .workers(3)
    .run()
    .await
}
