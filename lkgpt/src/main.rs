#![feature(ascii_char, async_closure)]
mod controls;
mod frame_capture;
mod llm;
mod server;
mod stt;
mod tts;
mod video;
// mod __;

use std::borrow::BorrowMut;

use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
        CreateChatCompletionRequestArgs,
    },
    Client,
};
use frame_capture::scene::SceneController;
use livekit::{publication::LocalTrackPublication, Room};
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

use async_openai::Client as OPENAI_CLIENT;

use bevy::{
    app::ScheduleRunnerPlugin, core::Name, core_pipeline::tonemapping::Tonemapping, log::LogPlugin,
    prelude::*, render::renderer::RenderDevice, time::common_conditions::on_timer,
};
use bevy_panorbit_camera::{PanOrbitCamera, PanOrbitCameraPlugin};

use bevy_gaussian_splatting::{
    random_gaussians, utils::get_arg, GaussianCloud, GaussianSplattingBundle,
    GaussianSplattingPlugin,
};

use stt::STT;

use futures::StreamExt;
use livekit::{
    track::RemoteTrack,
    webrtc::{audio_stream::native::NativeAudioStream, video_stream::native::NativeVideoStream},
    DataPacketKind, RoomEvent,
};
use log::{error, warn};
use serde::{Deserialize, Serialize};

use crate::{
    controls::WorldControlChannel, llm::LLMChannel, server::RoomData, stt::AudioChannel,
    video::VideoChannel,
};

pub const LIVEKIT_API_SECRET_ENV: &str = "LIVEKIT_API_SECRET";
pub const LIVEKIT_API_KEY_ENV: &str = "LIVEKIT_API_KEY";
pub const LIVEKIT_WS_URL_ENV: &str = "LIVEKIT_WS_URL";
pub const OPENAI_ORG_ID_ENV: &str = "OPENAI_ORG_ID";
pub const DEEPGRAM_API_KEY_ENV: &str = "DEEPGRAM_API_KEY";
pub const ELEVENLABS_API_KEY_ENV: &str = "ELEVENLABS_API_KEY";

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
    image: image::ImageBuffer<image::Rgba<u8>, Vec<u8>>,
    framebuffer: std::sync::Arc<parking_lot::Mutex<Vec<u8>>>,
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
pub struct AppStateSync {
    state: std::sync::Arc<parking_lot::Mutex<ParticipantRoomName>>,
    dirty: bool,
}

#[derive(Resource)]
pub struct LivekitRoom {
    room: std::sync::Arc<Room>,
    room_events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
}

// SYSTEM
pub fn handle_room_events(
    async_runtime: Res<AsyncRuntime>,
    llm_channel: Res<llm::LLMChannel>,
    audio_channel: Res<stt::AudioChannel>,
    video_channel: Res<video::VideoChannel>,
    mut room_events: ResMut<LivekitRoom>,
) {
    while let Ok(event) = room_events.room_events.try_recv() {
        match event {
            RoomEvent::TrackSubscribed { track, publication: _, participant: _user } => {
                let rt = async_runtime.rt.clone();
                match track {
                    RemoteTrack::Audio(audio_track) => {
                        let audio_rtc_track = audio_track.rtc_track();
                        let mut audio_stream = NativeAudioStream::new(audio_rtc_track);
                        let audio_channel_tx = audio_channel.tx.clone();
                        rt.spawn(async move {
                            while let Some(frame) = audio_stream.next().await {
                                if let Err(e) = audio_channel_tx.send(frame.data.to_vec()) {
                                    log::error!("Couldn't send audio frame to stt {e}");
                                };
                            }
                        });
                    },
                    RemoteTrack::Video(video_track) => {
                        let video_rtc_track = video_track.rtc_track();
                        let mut video_stream = NativeVideoStream::new(video_rtc_track);
                        rt.spawn(async move {
                            while let Some(frame) = video_stream.next().await {
                                let c = frame.buffer.to_i420();
                                // livekit::webrtc::native::yuv_helper::i420_to_rgba(
                                //     c,
                                //     frame.width,
                                //     frame.height,
                                //     frame.width,
                                //     frame.height,
                                //     video_track.rotation,
                                //     video_track.flip,
                                // );
                                // if let Err(e)= audio_channel_tx.send(frame.data.to_vec()){
                                //     log::error!("Couldn't send audio frame to stt {e}");
                                // };
                            }
                        });
                    },
                };
            },
            RoomEvent::DataReceived { payload, kind, participant: _user } => {
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
            // RoomEvents::TrackMuted {} =>{
            // }
            _ => info!("incoming event {:?}", event),
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
) -> Result<TracksPublicationData, RoomError> {
    let audio_src = NativeAudioSource::new(
        AudioSourceOptions::default(),
        STT::SAMPLE_RATE,
        STT::NUM_OF_CHANNELS,
    );

    let audio_track =
        LocalAudioTrack::create_audio_track(bot_name, RtcAudioSource::Native(audio_src.clone()));

    let (width, height) = (1920, 1080);
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
    mut gaussian_assets: ResMut<Assets<GaussianCloud>>,
    mut scene_controller: ResMut<frame_capture::scene::SceneController>,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
) {
    let remote_file = Some("");
    let cloud = match remote_file {
        Some(filename) => {
            log::info!("loading {}", filename);
            asset_server.load(filename.to_string())
        },
        None => {
            log::info!("using test model");
            gaussian_assets.add(GaussianCloud::test_model())
        },
    };

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
            camera: Camera { target: render_target, hdr: true, ..default() },
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
    mut next_state: ResMut<NextState<AppState>>,
    mut scene_controller: Res<SceneController>,
) {
    if !server_state_clone.dirty {
        let participant_room_name = &(server_state_clone.state.lock().0).clone();
        log::info!("participant_room_name {:#?}", &participant_room_name);
        if !participant_room_name.is_empty() {
            let rt = async_runtime.rt.clone();
            let video_frame_dimensions = scene_controller.dimensions();
            let status = rt.block_on(server::setup_and_connect_to_livekit(
                participant_room_name.clone(),
                video_frame_dimensions,
            ));
            match status {
                Ok(room_data) => {
                    info!("connected to livekit room");

                    let RoomData {
                        livekit_room,
                        stream_frame_data,
                        video_pub,
                        audio_src,
                        audio_pub,
                    } = room_data;

                    info!("initializing required bevy resources");
                    commands.init_resource::<LLMChannel>();
                    commands.init_resource::<WorldControlChannel>();
                    commands.init_resource::<AudioChannel>();
                    commands.init_resource::<STT>();
                    commands.init_resource::<VideoChannel>();
                    commands.insert_resource(stream_frame_data);
                    commands.insert_resource(livekit_room);
                    next_state.set(AppState::Active);
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
    pub single_image: bool,
}

fn main() {
    dotenvy::dotenv().ok();

    // ************** REQUIRED ENV VARS **************
    std::env::var(LIVEKIT_API_SECRET_ENV).expect("LIVEKIT_API_SECRET must be set");
    std::env::var(LIVEKIT_API_KEY_ENV).expect("LIVEKIT_API_KEY must be set");
    std::env::var(LIVEKIT_WS_URL_ENV).expect("LIVEKIT_WS_URL is not set");
    std::env::var(OPENAI_ORG_ID_ENV).expect("OPENAI_ORG_ID must be set");
    std::env::var(DEEPGRAM_API_KEY_ENV).expect("DEEPGRAM_API_KEY must be set");
    std::env::var(ELEVENLABS_API_KEY_ENV).expect("ELEVENLABS_API_KEY must be set");

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

    let config = AppConfig { width: 1920, height: 1080, single_image: true };

    // setup frame capture
    app.insert_resource(frame_capture::scene::SceneController::new(
        config.width,
        config.height,
        config.single_image,
    ));
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
    app.init_resource::<server::ActixServer>();
    app.init_resource::<frame_capture::scene::SceneController>();
    app.add_event::<frame_capture::scene::SceneController>();

    app.add_systems(Update, server::shutdown_bevy_remotely);
    app.add_systems(
        Update,
        handle_room_events
            .run_if(resource_exists::<llm::LLMChannel>())
            .run_if(resource_exists::<stt::AudioChannel>())
            .run_if(resource_exists::<video::VideoChannel>())
            .run_if(resource_exists::<LivekitRoom>()),
    );
    app.add_systems(
        Update,
        llm::run_llm
            .run_if(resource_exists::<llm::LLMChannel>())
            .run_if(in_state(AppState::Active)),
    );

    app.add_systems(
        Update,
        sync_bevy_and_server_resources.run_if(on_timer(std::time::Duration::from_secs(2))),
    );

    app.add_systems(OnEnter(AppState::Active), setup_gaussian_cloud);

    app.run();
}
