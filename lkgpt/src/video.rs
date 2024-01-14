use bevy::ecs::{
    system::Resource,
    world::{FromWorld, World},
};
use futures::StreamExt;
use livekit::webrtc::video_stream::native::NativeVideoStream;

pub struct ReceivedVideoFrame {
    pub image_buffer: Vec<u8>,
    pub timestamp: i64, // When the frame was captured in microseconds
}

#[derive(Resource)]
pub struct VideoChannel {
    pub tx: crossbeam_channel::Sender<Vec<i16>>,
    rx: crossbeam_channel::Receiver<Vec<i16>>,
}

impl FromWorld for VideoChannel {
    fn from_world(_: &mut World) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<i16>>();
        Self { tx, rx }
    }
}

async fn video_stream_handler(mut video: NativeVideoStream) {
    let mut counter = 0_u8;
    let max_fps = 10;

    while let Some(frame) = video.next().await {
        if counter % max_fps == 0 {
            log::info!("video frame info - {frame:#?}");
        }

        counter = (counter + 1) % max_fps;
    }
}
