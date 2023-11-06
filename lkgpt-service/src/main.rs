#![feature(async_closure)]
mod response;
mod routes;
mod state;

use actix_web::{middleware, web::Data, App, HttpServer};
use dotenv::dotenv;

use log::info;
use std::env;

use crate::{routes::top_level_routes, state::ServerState};

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    dotenv().ok();
    std::env::var("LIVEKIT_API_KEY").expect("LIVEKIT_API_KEY must be set");
    std::env::var("LIVEKIT_API_SECRET").expect("LIVEKIT_API_SECRET must be set");
    let port = env::var("PORT")
        .unwrap_or_else(|_| "4000".to_string())
        .parse::<u16>()?;

    pretty_env_logger::formatted_builder()
        .filter_module("actix_server", log::LevelFilter::Info)
        .filter_module("actix_web", log::LevelFilter::Info)
        .filter_module("vulkan_turbo", log::LevelFilter::Info)
        .filter_module("vulkan_core", log::LevelFilter::Info)
        .filter_module("lkgpt", log::LevelFilter::Info)
        .init();

    let server_data = Data::new(parking_lot::Mutex::new(ServerState::new()));

    info!("starting HTTP server on port {port}");

    HttpServer::new(move || {
        App::new()
            .wrap(middleware::Compress::default())
            .wrap(middleware::Logger::new("IP - %a | Time - %D ms"))
            .wrap(middleware::DefaultHeaders::new().add(("Content-Type", "application/json")))
            .app_data(server_data.clone())
            .configure(top_level_routes)
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await?;

    Ok(())
}
