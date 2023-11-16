use std::sync::Arc;

use chrono::TimeZone;
use futures::StreamExt;
use livekit::{
    track::RemoteTrack,
    webrtc::{audio_stream::native::NativeAudioStream, video_stream::native::NativeVideoStream},
    DataPacketKind, RoomEvent,
};
use log::{info, warn};
use serde::{Deserialize, Serialize};

use crate::stt::{transcribe, STT};

#[derive(Serialize, Deserialize)]
struct RoomText {
    message: String,
    timestamp: i64,
}

// impl Serialize for RoomText {
//     fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         S: serde::Serializer,
//     {
//         let mut s = serializer.serialize_struct("RoomText", 2)?;
//         s.serialize_field("message", &self.message)?;
//         let utc_timestamp = match Utc.timestamp_opt(self.timestamp, 0){
//             chrono::LocalResult::Single(time) => time.to_string(),
//             _ => {
//                 warn!("Couldn't convert text msg time to timestamp.");
//                 return serde::Serializer::Error()
//             }
//         };
//         s.serialize_field("timestamp", &utc_timestamp)?;
//         s.end()
//     }
// }

pub async fn handle_room_events(
    turbo_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
    // tts_client: Arc<STT>,
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
                    // tokio::spawn(transcribe(
                    //     turbo_input_tx.clone(),
                    //     tts_client.clone(),
                    //     audio_stream,
                    // ));
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
                info!("Data received");
                if kind == DataPacketKind::Reliable {
                    if let Some(payload) = payload.as_ascii() {
                        let room_text: serde_json::Result<RoomText> =
                            serde_json::from_str(payload.as_str());
                        match room_text {
                            Ok(room_text) => {
                                let mut msg = room_text.message;
                                msg.push(' ');
                                info!("MSG {msg}");
                                let _ = turbo_input_tx.send(msg);
                            }
                            Err(e) => {
                                warn!("Couldn't deserialize room text. {e:#?}");
                            }
                        }

                        info!("text from room {:#?}", payload.as_str());
                    }
                }
            }
            _ => info!("incoming event {:?}", event),
        }
    }
    warn!("\n*************** NO LONGER HANDLING ROOM EVENTS ***************");
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
