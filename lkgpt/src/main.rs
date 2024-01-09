#![feature(ascii_char, async_closure)]
mod frame_capture;
mod server;
mod stt;
mod tts;
// mod __;

use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
        CreateChatCompletionRequestArgs,
    },
    Client,
};
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
use server::http_server;
use tokio::sync::mpsc;

use crate::tts::TTS;

use futures::StreamExt;
use livekit::{
    track::RemoteTrack,
    webrtc::{audio_stream::native::NativeAudioStream, video_stream::native::NativeVideoStream},
    DataPacketKind, RoomEvent,
};
use log::{error, warn};
use serde::{Deserialize, Serialize};

use crate::stt::{transcribe, STT};

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
}

pub async fn handle_room_events(
    gpt_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
    stt_client: STT,
    mut room_events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
) -> anyhow::Result<()> {
    while let Some(event) = room_events.recv().await {
        match event {
            RoomEvent::TrackSubscribed { track, publication: _, participant: _user } => match track
            {
                RemoteTrack::Audio(audio_track) => {
                    let audio_rtc_track = audio_track.rtc_track();
                    let audio_stream = NativeAudioStream::new(audio_rtc_track);
                    let stt_client_for_thread = stt_client.clone();
                    tokio::spawn(transcribe(stt_client_for_thread, audio_stream));
                },
                RemoteTrack::Video(video_track) => {
                    let video_rtc_track = video_track.rtc_track();
                    let video_stream = NativeVideoStream::new(video_rtc_track);
                    tokio::spawn(video_stream_handler(video_stream));
                },
            },
            RoomEvent::DataReceived { payload, kind, participant: _user } => {
                if kind == DataPacketKind::Reliable {
                    if let Some(payload) = payload.as_ascii() {
                        let room_text: serde_json::Result<RoomText> =
                            serde_json::from_str(payload.as_str());
                        match room_text {
                            Ok(room_text) => {
                                if let Err(e) =
                                    gpt_input_tx.send(format!("[chat]{} ", room_text.message))
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
    Ok(())
}

pub struct TracksPublicationData {
    pub video_src: NativeVideoSource,
    pub video_pub: LocalTrackPublication,
    pub audio_src: NativeAudioSource,
    pub audio_pub: LocalTrackPublication,
}

pub async fn publish_tracks(
    room: std::sync::Arc<Room>,
) -> Result<TracksPublicationData, RoomError> {
    const BOT_NAME: &str = "kitt";

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

fn remove_prefix(s: &str, prefix: &str) -> String {
    let s = match s.strip_prefix(prefix) {
        Some(s) => s,
        None => s,
    };
    s.to_owned()
}

/// Stream text chunks to gpt as it's being generated, with <1s latency.
/// Note: if chunks don't end with space or punctuation (" ", ".", "?", "!"),
/// the stream will wait for more text.
/// Used during input streaming to chunk text blocks and set last char to space
pub async fn gpt(
    mut text_input_rx: mpsc::UnboundedReceiver<String>,
    openai_client: Client<OpenAIConfig>,
    mut tts_client: TTS,
) -> anyhow::Result<()> {
    let splitters = ['.', ',', '?', '!', ';', ':', '—', '-', ')', ']', '}', ' '];

    let mut txt_buffer = String::new();
    let mut tts_buffer = String::new();

    let mut req_args = CreateChatCompletionRequestArgs::default();
    let openai_req = req_args.model("gpt-4-1106-preview").max_tokens(512u16);

    let text_chat_prefix = "[chat]";
    // let text_latency = Duration::from_millis(500);
    while let Some(chunk) = text_input_rx.recv().await {
        txt_buffer.push_str(&chunk);
        if txt_buffer.starts_with(text_chat_prefix) || ends_with_splitter(&splitters, &txt_buffer) {
            let request = openai_req
                .messages([ChatCompletionRequestUserMessageArgs::default()
                    .content(ChatCompletionRequestUserMessageContent::Text(remove_prefix(
                        &txt_buffer,
                        text_chat_prefix,
                    )))
                    .build()?
                    .into()])
                .build()?;

            let mut gpt_resp_stream = openai_client.chat().create_stream(request).await?;
            while let Some(result) = gpt_resp_stream.next().await {
                match result {
                    Ok(response) => {
                        for chat_choice in response.choices {
                            if let Some(content) = chat_choice.delta.content {
                                tts_buffer.push_str(&content);
                                if ends_with_splitter(&splitters, &tts_buffer) {
                                    let msg = {
                                        let txt = tts_buffer.clone();
                                        txt.trim().to_owned()
                                    };
                                    if let Err(e) = tts_client.send(msg) {
                                        error!("Coudln't send gpt text chunk to tts channel - {e}");
                                    } else {
                                        tts_buffer.clear();
                                    };
                                }
                            };
                        }
                    },
                    Err(err) => {
                        warn!("chunk error: {err:#?}");
                    },
                }
            }
            txt_buffer.clear();
        }
    }
    Ok(())
}

fn ends_with_splitter(splitters: &[char], chunk: &str) -> bool {
    !chunk.is_empty() && chunk != " " && splitters.iter().any(|&splitter| chunk.ends_with(splitter))
}

fn setup_gaussian_cloud(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut gaussian_assets: ResMut<Assets<GaussianCloud>>,
    mut scene_controller: ResMut<frame_capture::scene::SceneController>,
    mut images: ResMut<Assets<Image>>,
    render_device: Res<RenderDevice>,
) {
    let cloud: Handle<GaussianCloud>;

    let file_arg = Some("1000".to_string());
    if let Some(n) = file_arg.clone().and_then(|s| s.parse::<usize>().ok()) {
        log::info!("generating {} gaussians", n);
        cloud = gaussian_assets.add(random_gaussians(n));
    } else if let Some(filename) = file_arg {
        log::info!("loading {}", filename);
        cloud = asset_server.load(filename.to_string());
    } else {
        log::info!("using test model");
        cloud = gaussian_assets.add(GaussianCloud::test_model());
    }

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

// pub fn sync_resource_and_state(
//     server_state_clone: Res<AppStateSync>,
//     mut next_state: ResMut<NextState<AppState>>,
//     curr_app_state: Res<State<AppState>>,
// ) {
//     let bevy_app_state = AppStateServerResource::from(*curr_app_state.get());
//     let svr_resource_state = server_state_clone.state.lock();

//     if *svr_resource_state != bevy_app_state && *svr_resource_state != AppStateServerResource::Init
//     {
//         log::info!("About to toggle bevy app state");
//         match *svr_resource_state {
//             AppStateServerResource::Active => {
//                 next_state.set(AppState::Active);
//                 log::info!("bevy app is now streaming");
//             },
//             AppStateServerResource::Idle => {
//                 next_state.set(AppState::Idle);
//                 log::info!("bevy app is NOT streaming");
//             },
//             _ => {},
//         }
//     }
// }

pub fn sync__bevy_resource_and_server_resource(
    server_state_clone: Res<AppStateSync>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    let svr_resource_state = server_state_clone.state.lock();
    let participant_room_name = svr_resource_state.0;
    if !participant_room_name.is_empty() {};
}

fn time_passed(t: f32) -> impl FnMut(Local<f32>, Res<Time>) -> bool {
    move |mut timer: Local<f32>, time: Res<Time>| {
        // Tick the timer
        *timer += time.delta_seconds();
        // Return true if the timer has passed the time
        *timer >= t
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
    std::env::var("LIVEKIT_API_SECRET").expect("LIVEKIT_API_SECRET must be set");
    std::env::var("LIVEKIT_API_KEY").expect("LIVEKIT_API_KEY must be set");
    std::env::var("LIVEKIT_WS_URL").expect("LIVEKIT_WS_URL is not set");
    std::env::var("OPENAI_ORG_ID").expect("OPENAI_ORG_ID must be set");
    std::env::var("DEEPGRAM_API_KEY").expect("DEEPGRAM_API_KEY must be set");
    std::env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY must be set");

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

    // app.add_systems(
    //     Update,
    //     sync_resource_and_state.run_if(time_passed(2.5)),
    // );
    app.add_systems(OnEnter(AppState::Active), setup_gaussian_cloud);

    // app.add_systems(Update, setup_gaussian_cloud);

    // app.add_systems(Update, || {
    //     info!("update");
    // });
    // app.add_systems(Startup, (setup_gaussian_cloud, handle_room_events));
    // app.add_systems(Update, gpt);

    app.run();
}

/*

use std::sync::Arc;

use anyhow::Result;
use async_openai::{config::OpenAIConfig, Client as OPENAI_CLIENT};
use livekit::{publication::LocalTrackPublication, Room};

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

use crate::stt::STT;

use livekit as lsdk;
use log::{error, info, warn};
use lsdk::RoomError;
use parking_lot::Mutex;
use tokio::{
    sync::mpsc::{Receiver, UnboundedReceiver},
    task::JoinHandle,
};

use crate::{
    gpt::gpt,
    room_events::handle_room_events,
    stt::STT,
    track_pub::{publish_tracks, TracksPublicationData},
    tts::TTS,
};

pub struct TurboLivekitConnector {
    room: Arc<Room>,
    text_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
    room_event_handle: JoinHandle<Result<()>>,
    video_pub: LocalTrackPublication,
    audio_pub: LocalTrackPublication,
    gpt_thread_handle: JoinHandle<()>,
    render_thread_handle: Option<JoinHandle<()>>,
}

impl TurboLivekitConnector {
    pub async fn new(participant_room_name: String) -> Result<Self> {
        // ************** REQUIRED ENV VARS **************
        let open_ai_org_id = std::env::var("OPENAI_ORG_ID").expect("OPENAI_ORG_ID must be");
        let lvkt_url = std::env::var("LIVEKIT_WS_URL").expect("LIVEKIT_WS_URL is not set");

        // ************** CONNECT TO ROOM **************
        let lvkt_token = utils::create_bot_token(participant_room_name, BOT_NAME)?;
        let room_options = lsdk::RoomOptions {
            ..Default::default()
        };
        let (room, room_events) = lsdk::Room::connect(&lvkt_url, &lvkt_token, room_options).await?;
        info!("Established connection with room. ID -> [{}]", room.name());
        let room = Arc::new(room);

        // ************** CREATE MESSAGING CHANNELS **************
        let (gpt_input_tx, gpt_input_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (to_voice_tx, from_gpt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        // ************** SETUP OPENAI, TTS, & STT **************
        let TracksPublicationData {
            video_pub,
            video_src,
            audio_src,
            audio_pub,
        } = publish_tracks(room.clone()).await?;

        let openai_client =
            OPENAI_CLIENT::with_config(OpenAIConfig::new().with_org_id(open_ai_org_id));

        let stt_client = STT::new(gpt_input_tx.clone()).await?;
        let mut tts_client = TTS::new()?;
        tts_client.setup_ws_client(audio_src).await?;

        // let tts_receiver_handle = tokio::spawn(tts_receiver(from_gpt_rx, tts_client_for_receiver));

        // let tts_thread_handle = tokio::spawn(tts.transcribe(main_input_rx));

        // let render_thread_handle = tokio::spawn(async move {
        //     if let Err(e) = turbo.render(video_src).await {
        //         error!("Turbo graphics render thread exited with error: {e}");
        //     }
        // });

        Ok(Self {
            room,
            text_input_tx: gpt_input_tx,
            audio_pub,
            video_pub,
            room_event_handle,
            gpt_thread_handle,
            render_thread_handle: Some(render_thread_handle),
        })
    }

    pub fn get_thread_handle(&mut self) -> JoinHandle<()> {
        self.render_thread_handle
            .take()
            .expect("render thread handle should not be None")
    }

    pub fn get_txt_input_sender(&mut self) -> tokio::sync::mpsc::UnboundedSender<String> {
        self.text_input_tx.clone()
    }

    async fn shutdown(&mut self) -> Result<(), RoomError> {
        self.room.close().await
    }
}
*/

/*

- set participant room,
- use watcher to update system state
-  try to connect to livekit
    -

*/
