mod response;
mod routes;
mod state;
mod utils;

use actix_web::{middleware, web::Data, App, HttpServer};

use log::info;
use std::env;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    dotenvy::dotenv().ok();

    std::env::var("LIVEKIT_API_KEY").expect("LIVEKIT_API_KEY must be set");
    std::env::var("LIVEKIT_API_SECRET").expect("LIVEKIT_API_SECRET must be set");
    let port = env::var("PORT")
        .unwrap_or_else(|_| "6669".to_string())
        .parse::<u16>()
        .expect("PORT couldn't be set");

    pretty_env_logger::formatted_builder()
        .filter_module("actix_server", log::LevelFilter::Info)
        .filter_module("actix_web", log::LevelFilter::Info)
        .filter_module("engine_livekit", log::LevelFilter::Info)
        .filter_module("livekit", log::LevelFilter::Info)
        .filter_module("her", log::LevelFilter::Info)
        .init();

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
