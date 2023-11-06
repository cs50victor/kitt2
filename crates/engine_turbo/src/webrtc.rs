use anyhow::Result;
use livekit::webrtc::video_source::native::NativeVideoSource;
use log::{info, warn};
use std::sync::mpsc::{self};
use tokio::task::JoinHandle;

use crate::Turbo;

pub struct TurboWebrtcConnector {
    render_input_sender: mpsc::Sender<&'static str>,
    render_thread_handle: Option<JoinHandle<()>>,
}

impl TurboWebrtcConnector {
    pub async fn new(livekit_vid_src: NativeVideoSource) -> Result<Self> {
        let (render_input_sender, render_input_receiver) = mpsc::channel::<&str>();

        let mut turbo = Turbo::new()?.load_basic_scene()?;

        let render_thread_handle = tokio::spawn(async move {
            match turbo.render(render_input_receiver, livekit_vid_src).await {
                Ok(_) => info!("Engine render thread exited in a smooth fashion"),
                Err(e) => info!("Engine render thread exited with error: {}", e),
            }
        });

        Ok(Self {
            render_input_sender,
            render_thread_handle: Some(render_thread_handle),
        })
    }

    pub fn get_render_thread_handle(&mut self) -> JoinHandle<()> {
        match self.render_thread_handle.take() {
            Some(render_thread_handle) => render_thread_handle,
            _ => panic!("render thread handle should not be None"),
        }
    }

    // TODO: remove static and use a proper lifetime
    pub fn get_render_input_sender(&mut self) -> mpsc::Sender<&'static str> {
        self.render_input_sender.clone()
    }
}

unsafe impl Send for TurboWebrtcConnector {}
unsafe impl Sync for TurboWebrtcConnector {}

impl Drop for TurboWebrtcConnector {
    fn drop(&mut self) {
        warn!("DROPPED - TURBOWEBRTC CONNECTOR ");
    }
}
