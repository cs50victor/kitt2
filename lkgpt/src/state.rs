use log::{error, info};

use crate::webrtc::TurboLivekitConnector;

pub type ServerStateMutex = parking_lot::Mutex<ServerState>;

#[derive(Default)]
pub struct ServerState {
    pub turbo_input_tx: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    pub turbo_livekit_connector_handle: Option<TurboLivekitConnector>,
}

impl ServerState {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Drop for ServerState {
    fn drop(&mut self) {
        if let Some(turbo_input_tx) = self.turbo_input_tx.take() {
            match turbo_input_tx.send("Goodbye".to_owned()) {
                Ok(_) => info!("Turbo Renderer should be exiting..."),
                Err(e) => error!("Error closing renderer: {e}"),
            };
        }

        if let Some(render_thread_handle) = self.turbo_livekit_connector_handle.take() {
            drop(render_thread_handle);
        }
    }
}
