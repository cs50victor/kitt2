use std::sync::Arc;

use chrono::TimeZone;
use futures::StreamExt;
use livekit::{
    track::RemoteTrack,
    webrtc::{audio_stream::native::NativeAudioStream, video_stream::native::NativeVideoStream},
    DataPacketKind, RoomEvent,
};
use log::{info, warn, error};
use parking_lot::{Mutex, MutexGuard, RawMutex};
use serde::{Deserialize, Serialize};

use crate::stt::{STT, transcribe};

#[derive(Serialize, Deserialize)]
struct RoomText {
    message: String,
    timestamp: i64,
}

pub async fn handle_room_events(
    gpt_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
    stt_client: STT,
    mut room_events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
) -> anyhow::Result<()> {
    while let Some(event) = room_events.recv().await {
        match event {
            RoomEvent::TrackSubscribed {
                track,
                publication: _,
                participant: _user,
            } => match track {
                RemoteTrack::Audio(audio_track) => {
                    let audio_rtc_track = audio_track.rtc_track();
                    let audio_stream = NativeAudioStream::new(audio_rtc_track);
                    let stt_client_for_thread = stt_client.clone();
                    tokio::spawn(transcribe(stt_client_for_thread, audio_stream));
                }
                RemoteTrack::Video(video_track) => {
                    let video_rtc_track = video_track.rtc_track();
                    let video_stream = NativeVideoStream::new(video_rtc_track);
                    tokio::spawn(video_stream_handler(video_stream));
                }
            },
            RoomEvent::DataReceived {
                payload,
                kind,
                participant: _user,
            } => {
                if kind == DataPacketKind::Reliable {
                    if let Some(payload) = payload.as_ascii() {
                        let room_text: serde_json::Result<RoomText> =
                            serde_json::from_str(payload.as_str());
                        match room_text {
                            Ok(room_text) => {
                                if let Err(e) = gpt_input_tx.send(format!("{} ",room_text.message)){
                                    error!("Couldn't send the text to gpt {e}")
                                };
                            }
                            Err(e) => {
                                warn!("Couldn't deserialize room text. {e:#?}");
                            }
                        }

                        info!("text from room {:#?}", payload.as_str());
                    }
                }
            }
            // RoomEvents::TrackMuted {} =>{

            // }
            _ => info!("incoming event {:?}", event),
        }
    }
    Ok(())
}

async fn video_stream_handler(mut video: NativeVideoStream) {
    let mut counter = 0_u8;
    let max_fps = 10;

    while let Some(frame) = video.next().await {
        if counter % max_fps == 0 {
            info!("video frame info - {frame:#?}");
        }

        counter = (counter + 1) % max_fps;
    }
}
