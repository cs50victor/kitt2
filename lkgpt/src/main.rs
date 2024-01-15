#![feature(ascii_char, async_closure, slice_pattern)]
mod controls;
mod frame_capture;
mod llm;
mod server;
mod stt;
mod tts;
mod video;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use frame_capture::scene::SceneController;
use image::RgbaImage;
use livekit::{publication::LocalTrackPublication, webrtc::video_frame::VideoBuffer, Room};
// use actix_web::{middleware, web::Data, App, HttpServer};
use log::info;

use livekit::{
    options::{TrackPublishOptions, VideoCodec},
    track::{LocalAudioTrack, LocalTrack, LocalVideoTrack, TrackSource},
    webrtc::{
        audio_source::native::NativeAudioSource,
        prelude::{AudioSourceOptions, RtcAudioSource},
        video_source::{native::NativeVideoSource, RtcVideoSource},
    },
    RoomError,
};

use bevy::{
    app::ScheduleRunnerPlugin, core::Name, core_pipeline::tonemapping::Tonemapping, log::LogPlugin,
    prelude::*, render::renderer::RenderDevice, time::common_conditions::on_timer,
};
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraPlugin};

use bevy_gaussian_splatting::{GaussianCloud, GaussianSplattingBundle, GaussianSplattingPlugin};

use pollster::FutureExt;

use futures::StreamExt;
use livekit::{
    track::RemoteTrack,
    webrtc::{audio_stream::native::NativeAudioStream, video_stream::native::NativeVideoStream},
    DataPacketKind, RoomEvent,
};
use log::{error, warn};
use rodio::cpal::Sample as _;
use serde::{Deserialize, Serialize};
use stt::STT;

use crate::{
    controls::WorldControlChannel, llm::LLMChannel, server::RoomData, tts::TTS, video::VideoChannel,
};

pub const LIVEKIT_API_SECRET: &str = "LIVEKIT_API_SECRET";
pub const LIVEKIT_API_KEY: &str = "LIVEKIT_API_KEY";
pub const LIVEKIT_WS_URL: &str = "LIVEKIT_WS_URL";
pub const OPENAI_ORG_ID: &str = "OPENAI_ORG_ID";
pub const DEEPGRAM_API_KEY: &str = "DEEPGRAM_API_KEY";
pub const ELEVENLABS_API_KEY: &str = "ELEVENLABS_API_KEY";

#[derive(Resource)]
pub struct AsyncRuntime {
    rt: std::sync::Arc<tokio::runtime::Runtime>,
}

impl FromWorld for AsyncRuntime {
    fn from_world(_world: &mut World) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        AsyncRuntime { rt: std::sync::Arc::new(rt) }
    }
}

#[derive(Clone)]
struct FrameData {
    video_frame: std::sync::Arc<
        parking_lot::Mutex<
            livekit::webrtc::video_frame::VideoFrame<livekit::webrtc::video_frame::I420Buffer>,
        >,
    >,
}

#[derive(Resource)]
pub struct StreamingFrameData {
    pixel_size: u32,
    frame_data: FrameData,
    video_src: NativeVideoSource,
}

#[derive(Serialize, Deserialize)]
struct RoomText {
    message: String,
    timestamp: i64,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq, Hash, States)]
pub enum AppState {
    #[default]
    Idle,
    Active,
}

#[derive(Default, Debug, PartialEq)]
pub enum AppStateServerResource {
    #[default]
    Init,
    Idle,
    Active,
}

#[derive(Default, Debug, PartialEq)]
struct ParticipantRoomName(String);

impl From<AppState> for AppStateServerResource {
    fn from(value: AppState) -> Self {
        match value {
            AppState::Idle => AppStateServerResource::Idle,
            AppState::Active => AppStateServerResource::Active,
        }
    }
}

#[derive(Resource)]
pub struct AudioSync {
    should_stop: Arc<AtomicBool>,
}

#[derive(Resource)]
pub struct AppStateSync {
    state: std::sync::Arc<parking_lot::Mutex<ParticipantRoomName>>,
    dirty: bool,
}

#[derive(Resource)]
pub struct LivekitRoom {
    #[allow(dead_code)]
    room: std::sync::Arc<Room>,
    room_events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
}

// SYSTEM
pub fn handle_room_events(
    async_runtime: Res<AsyncRuntime>,
    llm_channel: Res<llm::LLMChannel>,
    stt_client: ResMut<STT>,
    _video_channel: Res<video::VideoChannel>,
    audio_syncer: ResMut<AudioSync>,
    mut room_events: ResMut<LivekitRoom>,
    _single_frame_data: ResMut<crate::StreamingFrameData>,
) {
    while let Ok(event) = room_events.room_events.try_recv() {
        println!("\n\n🤡received room event {:?}", event);
        match event {
            RoomEvent::TrackSubscribed { track, publication: _, participant: _user } => {
                match track {
                    RemoteTrack::Audio(audio_track) => {
                        let audio_rtc_track = audio_track.rtc_track();
                        let mut audio_stream = NativeAudioStream::new(audio_rtc_track);
                        let audio_should_stop = audio_syncer.should_stop.clone();
                        let stt_client = stt_client.clone();
                        async_runtime.rt.spawn(async move {
                            while let Some(frame) = audio_stream.next().await {
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
                        });
                    },
                    RemoteTrack::Video(video_track) => {
                        let video_rtc_track = video_track.rtc_track();
                        let pixel_size = 4;
                        let mut video_stream = NativeVideoStream::new(video_rtc_track);

                        async_runtime.rt.spawn(async move {
                            // every 10 video frames
                            let mut i = 0;
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
                                    llm_channel.tx.send(format!("[chat]{} ", room_text.message))
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
                audio_syncer.should_stop.store(true, Ordering::Relaxed);
            },
            RoomEvent::TrackUnmuted { participant: _, publication: _ } => {
                audio_syncer.should_stop.store(false, Ordering::Relaxed);
            },
            // RoomEvent::ActiveSpeakersChanged { speakers } => {
            //     if speakers.is_empty() {
            //         audio_syncer.should_stop.store(true, Ordering::Relaxed);
            //     }
            //     let is_main_participant_muted = speakers.iter().any(|speaker| speaker.name() != "kitt");
            //     audio_syncer.should_stop.store(is_main_participant_muted, Ordering::Relaxed);
            // }
            _ => info!("received room event {:?}", event),
        }
    }
}

pub struct TracksPublicationData {
    pub video_src: NativeVideoSource,
    pub video_pub: LocalTrackPublication,
    pub audio_src: NativeAudioSource,
    pub audio_pub: LocalTrackPublication,
}

pub async fn publish_tracks(
    room: std::sync::Arc<Room>,
    bot_name: &str,
    video_frame_dimension: (u32, u32),
) -> Result<TracksPublicationData, RoomError> {
    let audio_src = NativeAudioSource::new(
        AudioSourceOptions::default(),
        TTS::SAMPLE_RATE,
        TTS::NUM_OF_CHANNELS,
    );

    let audio_track =
        LocalAudioTrack::create_audio_track(bot_name, RtcAudioSource::Native(audio_src.clone()));

    let (width, height) = video_frame_dimension;
    let video_src =
        NativeVideoSource::new(livekit::webrtc::video_source::VideoResolution { width, height });
    let video_track =
        LocalVideoTrack::create_video_track(bot_name, RtcVideoSource::Native(video_src.clone()));

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

fn setup_gaussian_cloud(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    _gaussian_assets: ResMut<Assets<GaussianCloud>>,
    mut scene_controller: ResMut<frame_capture::scene::SceneController>,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
) {
    // let remote_file = Some("https://huggingface.co/datasets/cs50victor/splats/resolve/main/train/point_cloud/iteration_7000/point_cloud.gcloud");
    // TODO: figure out how to load remote files later
    let splat_file = "splats/train/point_cloud/iteration_7000/point_cloud.gcloud";
    log::info!("loading {}", splat_file);
    let cloud = asset_server.load(splat_file.to_string());

    // let cloud = gaussian_assets.add(GaussianCloud::test_model());

    let render_target = frame_capture::scene::setup_render_target(
        &mut commands,
        &mut images,
        &render_device,
        &mut scene_controller,
        15,
        String::from("main_scene"),
    );

    commands.spawn((GaussianSplattingBundle { cloud, ..default() }, Name::new("gaussian_cloud")));

    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_translation(Vec3::new(0.0, 1.5, 5.0)),
            tonemapping: Tonemapping::None,
            camera: Camera { target: render_target, ..default() },
            ..default()
        },
        PanOrbitCamera {
            allow_upside_down: true,
            orbit_smoothness: 0.0,
            pan_smoothness: 0.0,
            zoom_smoothness: 0.0,
            ..default()
        },
    ));
}

pub fn sync_bevy_and_server_resources(
    mut commands: Commands,
    async_runtime: Res<AsyncRuntime>,
    mut server_state_clone: ResMut<AppStateSync>,
    mut set_app_state: ResMut<NextState<AppState>>,
    scene_controller: Res<SceneController>,
) {
    if !server_state_clone.dirty {
        let participant_room_name = &(server_state_clone.state.lock().0).clone();
        if !participant_room_name.is_empty() {
            let video_frame_dimensions = scene_controller.dimensions();
            let status = async_runtime.rt.block_on(server::setup_and_connect_to_livekit(
                participant_room_name.clone(),
                video_frame_dimensions,
            ));
            match status {
                Ok(room_data) => {
                    info!("🎉connected to livekit room");

                    let RoomData {
                        livekit_room,
                        stream_frame_data,
                        audio_src,
                        bot_name: _,
                        video_pub: _,
                        audio_pub: _,
                    } = room_data;

                    info!("initializing required bevy resources");

                    let tts = async_runtime.rt.block_on(TTS::new(audio_src)).unwrap();
                    let llm_channel = LLMChannel::new();
                    let llm_tx = llm_channel.tx.clone();

                    commands.insert_resource(llm_channel);
                    commands.init_resource::<WorldControlChannel>();

                    let stt = async_runtime.rt.block_on(STT::new(llm_tx)).unwrap();
                    commands.insert_resource(stt);

                    commands.init_resource::<VideoChannel>();
                    commands.insert_resource(tts);
                    commands.insert_resource(stream_frame_data);
                    commands.insert_resource(livekit_room);

                    set_app_state.set(AppState::Active);
                    server_state_clone.dirty = true;
                },
                Err(e) => {
                    info!("couldn't connect to livekit room {e:#?}");
                },
            }
        };
    }
}

pub struct AppConfig {
    pub width: u32,
    pub height: u32,
}

fn main() {
    dotenvy::from_filename_override(".env.local").ok();

    // ************** REQUIRED ENV VARS **************
    std::env::var(LIVEKIT_API_SECRET).expect("LIVEKIT_API_SECRET must be set");
    std::env::var(LIVEKIT_API_KEY).expect("LIVEKIT_API_KEY must be set");
    std::env::var(LIVEKIT_WS_URL).expect("LIVEKIT_WS_URL is not set");
    std::env::var(OPENAI_ORG_ID).expect("OPENAI_ORG_ID must be set");
    std::env::var(DEEPGRAM_API_KEY).expect("DEEPGRAM_API_KEY must be set");
    std::env::var(ELEVENLABS_API_KEY).expect("ELEVENLABS_API_KEY must be set");

    let mut formatted_builder = pretty_env_logger::formatted_builder();

    let pretty_env_builder = formatted_builder
        .filter_module("lkgpt", log::LevelFilter::Info)
        .filter_module("actix_server", log::LevelFilter::Info)
        .filter_module("bevy", log::LevelFilter::Info)
        .filter_module("actix_web", log::LevelFilter::Info);

    if cfg!(target_os = "unix") {
        pretty_env_builder.filter_module("livekit", log::LevelFilter::Info);
    }

    pretty_env_builder.init();

    let mut app = App::new();

    let config = AppConfig { width: 1920, height: 1080 };

    app.insert_resource(frame_capture::scene::SceneController::new(config.width, config.height));
    app.insert_resource(ClearColor(Color::rgb_u8(0, 0, 0)));

    app.add_plugins((
        bevy_web_asset::WebAssetPlugin,
        DefaultPlugins
            .set(ImagePlugin::default_nearest())
            // "headless" window
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                close_when_requested: false,
            }).disable::<LogPlugin>(),
        frame_capture::image_copy::ImageCopyPlugin,
        frame_capture::scene::CaptureFramePlugin,
        ScheduleRunnerPlugin::run_loop(std::time::Duration::from_secs_f64(1.0 / 60.0)),
        PanOrbitCameraPlugin,
        // plugin for gaussian splatting
        GaussianSplattingPlugin,
    ));

    app.add_state::<AppState>();
    app.init_resource::<AsyncRuntime>();
    app.insert_resource(AudioSync { should_stop: Arc::new(AtomicBool::new(false)) });
    app.init_resource::<server::ActixServer>();

    app.init_resource::<frame_capture::scene::SceneController>();
    app.add_event::<frame_capture::scene::SceneController>();

    // app.add_systems(Update, move_camera);

    app.add_systems(Update, server::shutdown_bevy_remotely);

    app.add_systems(
        Update,
        handle_room_events
            .run_if(resource_exists::<llm::LLMChannel>())
            .run_if(resource_exists::<stt::STT>())
            .run_if(resource_exists::<video::VideoChannel>())
            .run_if(resource_exists::<LivekitRoom>()),
    );

    app.add_systems(
        Update,
        llm::run_llm
            .run_if(resource_exists::<llm::LLMChannel>())
            .run_if(resource_exists::<tts::TTS>())
            .run_if(in_state(AppState::Active)),
    );

    app.add_systems(
        Update,
        sync_bevy_and_server_resources.run_if(on_timer(std::time::Duration::from_secs(2))),
    );

    // app.add_systems(OnEnter(AppState::Active), setup_gaussian_cloud);

    app.run();
}

fn move_camera(mut camera: Query<&mut Transform, With<Camera>>) {
    for mut transform in camera.iter_mut() {
        transform.translation.x += 5.0;
    }
}
