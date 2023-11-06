use anyhow::Error;
use log::{error, info};

pub type ServerStateMutex = parking_lot::Mutex<ServerState>;

#[derive(Default)]
pub struct ServerState {
    pub render_input_sender: Option<std::sync::mpsc::Sender<&'static str>>,
    pub turbo_running: bool,
    pub room_msg_thread_handler: Option<tokio::task::JoinHandle<()>>,
    pub render_thread_handle: Option<std::thread::JoinHandle<()>>,
    pub turbo_webrtc_connector_handle:
        Option<tokio::task::JoinHandle<Result<engine_turbo::TurboWebrtcConnector, Error>>>,
}

impl ServerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn shutdown_turbo(&mut self) {
        if let Some(renderer_sender) = self.render_input_sender.take() {
            match renderer_sender.send("exit") {
                Ok(_) => info!("Turbo Renderer should be exiting..."),
                Err(e) => error!("Error closing renderer: {}", e),
            };
        }

        if let Some(render_thread_handle) = self.render_thread_handle.take() {
            match render_thread_handle.join() {
                Ok(_) => info!("Turbo Renderer thread successfully joined"),
                Err(e) => error!("Error joining render thread: {:#?}", e),
            };
        }

        if let Some(frame_encoder_thread_handle) = self.room_msg_thread_handler.take() {
            match frame_encoder_thread_handle.await {
                Ok(_) => info!("Frame encoder thread successfully joined"),
                Err(e) => error!("Error joining frame encoder thread: {:#?}", e),
            };
        }

        self.turbo_running = false;
    }
}

impl Drop for ServerState {
    fn drop(&mut self) {
        futures::executor::block_on(self.shutdown_turbo());
    }
}

unsafe impl Send for ServerState {}
unsafe impl Sync for ServerState {}
