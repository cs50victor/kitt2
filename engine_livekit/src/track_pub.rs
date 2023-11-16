use std::sync::Arc;

use livekit::{
    options::{TrackPublishOptions, VideoCodec},
    publication::LocalTrackPublication,
    track::{LocalAudioTrack, LocalTrack, LocalVideoTrack, TrackSource},
    webrtc::{
        audio_source::native::NativeAudioSource,
        prelude::{AudioSourceOptions, RtcAudioSource},
        video_source::{native::NativeVideoSource, RtcVideoSource},
    },
    Room, RoomError,
};

use crate::stt::STT;

pub struct TracksPublicationData {
    pub video_src: NativeVideoSource,
    pub video_pub: LocalTrackPublication,
    pub audio_src: NativeAudioSource,
    pub audio_pub: LocalTrackPublication,
}
const BOT_NAME: &str = "donut";

pub async fn publish_tracks(room: Arc<Room>) -> Result<TracksPublicationData, RoomError> {
    let audio_src = NativeAudioSource::new(
        AudioSourceOptions::default(),
        STT::SAMPLE_RATE,
        STT::NUM_OF_CHANNELS,
    );
    let audio_track =
        LocalAudioTrack::create_audio_track(BOT_NAME, RtcAudioSource::Native(audio_src.clone()));

    // TODO: Remove from here and import from Turbo. Resolution{} ?
    let (width, height) = (1920, 1080);
    let video_src =
        NativeVideoSource::new(livekit::webrtc::video_source::VideoResolution { width, height });
    let video_track =
        LocalVideoTrack::create_video_track(BOT_NAME, RtcVideoSource::Native(video_src.clone()));

    let video_publication = room
        .local_participant()
        .publish_track(
            LocalTrack::Video(video_track),
            TrackPublishOptions {
                source: TrackSource::Camera,
                video_codec: VideoCodec::VP8,
                ..Default::default()
            },
        )
        .await;
    let audio_publication = room
        .local_participant()
        .publish_track(
            LocalTrack::Audio(audio_track),
            TrackPublishOptions { source: TrackSource::Microphone, ..Default::default() },
        )
        .await;

    let video_pub = video_publication?;
    let audio_pub = audio_publication?;
    Ok(TracksPublicationData { video_src, video_pub, audio_src, audio_pub })
}
