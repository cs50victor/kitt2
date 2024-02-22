use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use bevy::ecs::system::{Res, ResMut};
use futures::{future, FutureExt, StreamExt};
use image::RgbaImage;
use livekit::{
    track::RemoteTrack,
    webrtc::{
        audio_stream::native::NativeAudioStream, video_frame::VideoBuffer,
        video_stream::native::NativeVideoStream,
    },
    DataPacketKind, RoomEvent,
};
use log::{error, info, warn};
use rodio::cpal::Sample;
use tokio::runtime::Runtime;

use crate::{llm, stt::STT, video, AsyncRuntime, AudioSync, LivekitRoom, RoomText};

async fn handle_video(mut video_stream: NativeVideoStream, pixel_size: u32) {
    // every 10 video frames
    let mut i = 0;
    info!("📸 handling video");
    while let Some(frame) = video_stream.next().await {
        log::error!("🤡received video frame | {:#?}", frame);
        // VIDEO FRAME BUFFER (i420_buffer)
        let video_frame_buffer = frame.buffer.to_i420();
        let width = video_frame_buffer.width();
        let height = video_frame_buffer.height();
        let rgba_stride = video_frame_buffer.width() * pixel_size;

        let (stride_y, stride_u, stride_v) = video_frame_buffer.strides();
        let (data_y, data_u, data_v) = video_frame_buffer.data();

        let rgba_buffer = RgbaImage::new(width, height);
        let rgba_raw = unsafe {
            std::slice::from_raw_parts_mut(
                rgba_buffer.as_raw().as_ptr() as *mut u8,
                rgba_buffer.len(),
            )
        };

        livekit::webrtc::native::yuv_helper::i420_to_rgba(
            data_y,
            stride_y,
            data_u,
            stride_u,
            data_v,
            stride_v,
            rgba_raw,
            rgba_stride,
            video_frame_buffer.width() as i32,
            video_frame_buffer.height() as i32,
        );

        if let Err(e) = rgba_buffer.save(format!("camera/{i}.png")) {
            log::error!("Couldn't save video frame {e}");
        };
        i += 1;
    }
    info!("🤡ended video thread");
}

pub async fn handle_room_events(
    async_runtime: Arc<Runtime>,
    llm_channel_tx: crossbeam_channel::Sender<String>,
    stt_client: STT,
    _video_channel: video::VideoChannel,
    should_stop: Arc<AtomicBool>,
    mut room_events: LivekitRoom,
    pixel_size: u32,
) {
    while let Some(event) = room_events.room_events.recv().await {
        println!("\n\n🤡received room event {:?}", event);
        match event {
            RoomEvent::TrackSubscribed { track, publication: _, participant: _user } => {
                match track {
                    RemoteTrack::Audio(audio_track) => {
                        let audio_rtc_track = audio_track.rtc_track();
                        let mut audio_stream = NativeAudioStream::new(audio_rtc_track);
                        let audio_should_stop = should_stop.clone();
                        let stt_client = stt_client.clone();
                        let rt = async_runtime.clone();

                        std::thread::spawn(move || {
                            while let Some(frame) = rt.block_on(audio_stream.next()) {
                                if audio_should_stop.load(Ordering::Relaxed) {
                                    continue;
                                }

                                let audio_buffer = frame
                                    .data
                                    .iter()
                                    .map(|sample| sample.to_sample::<u8>())
                                    .collect::<Vec<u8>>();

                                if audio_buffer.is_empty() {
                                    warn!("empty audio frame | {:#?}", audio_buffer);
                                    continue;
                                }

                                if let Err(e) = stt_client.send(audio_buffer) {
                                    error!("Couldn't send audio frame to stt {e}");
                                };
                            }
                            error!("audio thread ended");
                        });
                    },
                    RemoteTrack::Video(video_track) => {
                        let video_rtc_track = video_track.rtc_track();
                        let mut video_stream = NativeVideoStream::new(video_rtc_track);
                        let rt = async_runtime.clone();

                        std::thread::spawn(move || {
                            // every 10 video frames
                            let mut i = 0;
                            info!("📸 handling video");
                            loop {
                                while let Some(Some(frame)) = video_stream.next().now_or_never() {
                                    log::error!("🤡received video frame | {:#?}", frame);
                                    // VIDEO FRAME BUFFER (i420_buffer)
                                    let video_frame_buffer = frame.buffer.to_i420();
                                    let width = video_frame_buffer.width();
                                    let height = video_frame_buffer.height();
                                    let rgba_stride = video_frame_buffer.width() * pixel_size;

                                    let (stride_y, stride_u, stride_v) =
                                        video_frame_buffer.strides();
                                    let (data_y, data_u, data_v) = video_frame_buffer.data();

                                    let rgba_buffer = RgbaImage::new(width, height);
                                    let rgba_raw = unsafe {
                                        std::slice::from_raw_parts_mut(
                                            rgba_buffer.as_raw().as_ptr() as *mut u8,
                                            rgba_buffer.len(),
                                        )
                                    };

                                    livekit::webrtc::native::yuv_helper::i420_to_rgba(
                                        data_y,
                                        stride_y,
                                        data_u,
                                        stride_u,
                                        data_v,
                                        stride_v,
                                        rgba_raw,
                                        rgba_stride,
                                        video_frame_buffer.width() as i32,
                                        video_frame_buffer.height() as i32,
                                    );

                                    if let Err(e) = rgba_buffer.save(format!("camera/{i}.png")) {
                                        log::error!("Couldn't save video frame {e}");
                                    };
                                    i += 1;
                                }
                            }
                            info!("🤡ended video thread");
                        });
                    },
                };
            },
            RoomEvent::DataReceived { payload, kind, topic: _, participant: _ } => {
                if kind == DataPacketKind::Reliable {
                    if let Some(payload) = payload.as_ascii() {
                        let room_text: serde_json::Result<RoomText> =
                            serde_json::from_str(payload.as_str());
                        match room_text {
                            Ok(room_text) => {
                                if let Err(e) =
                                    llm_channel_tx.send(format!("[chat]{} ", room_text.message))
                                {
                                    error!("Couldn't send the text to gpt {e}")
                                };
                            },
                            Err(e) => {
                                warn!("Couldn't deserialize room text. {e:#?}");
                            },
                        }

                        info!("text from room {:#?}", payload.as_str());
                    }
                }
            },
            // ignoring the participant for now, currently assuming there is only one participant
            RoomEvent::TrackMuted { participant: _, publication: _ } => {
                should_stop.store(true, Ordering::Relaxed);
            },
            RoomEvent::TrackUnmuted { participant: _, publication: _ } => {
                should_stop.store(false, Ordering::Relaxed);
            },
            // RoomEvent::ActiveSpeakersChanged { speakers } => {
            //     if speakers.is_empty() {
            //         should_stop.store(true, Ordering::Relaxed);
            //     }
            //     let is_main_participant_muted = speakers.iter().any(|speaker| speaker.name() != "kitt");
            //     should_stop.store(is_main_participant_muted, Ordering::Relaxed);
            // }
            RoomEvent::ConnectionQualityChanged { quality: _, participant: _ } => {},
            _ => info!("received room event {:?}", event),
        }
    }
}
