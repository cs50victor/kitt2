use std::sync::Arc;

use anyhow::Result;
use async_openai::{config::OpenAIConfig, Client as OPENAI_CLIENT};
use livekit::{publication::LocalTrackPublication, Room};

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
    turbo::Turbo,
    utils,
};

pub struct TurboLivekitConnector {
    room: Arc<Room>,
    text_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
    cmd_input_sender: std::sync::mpsc::Sender<String>,
    room_event_handle: JoinHandle<Result<()>>,
    video_pub: LocalTrackPublication,
    audio_pub: LocalTrackPublication,
    gpt_thread_handle: JoinHandle<()>,
    render_thread_handle: Option<JoinHandle<()>>,
}

const BOT_NAME: &str = "talking_donut";

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
        let (cmd_input_sender, cmd_input_receiver) = std::sync::mpsc::channel::<String>();
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
        let mut turbo = Turbo::new()?.load_basic_scene()?;
        let stt_cleint = STT::new(gpt_input_tx.clone()).await?;
        let mut tts_client = TTS::new()?;
        tts_client.setup_ws_client(audio_src).await?;

        // ************** CREATE THREADS TO KICK THINGS OFF **************
        let room_event_handle = tokio::spawn(handle_room_events(
            gpt_input_tx.clone(),
            stt_cleint,
            room_events,
        ));

        // let tts_receiver_handle = tokio::spawn(tts_receiver(from_gpt_rx, tts_client_for_receiver));

        // let tts_thread_handle = tokio::spawn(tts.transcribe(main_input_rx));

        let gpt_thread_handle = tokio::spawn(async {
            if let Err(e) = gpt(gpt_input_rx, openai_client, tts_client).await {
                error!("GPT thread exited with error: {e}");
            }
        });

        let render_thread_handle = tokio::spawn(async move {
            if let Err(e) = turbo.render(video_src).await {
                error!("Turbo graphics render thread exited with error: {e}");
            }
        });

        Ok(Self {
            room,
            text_input_tx: gpt_input_tx,
            audio_pub,
            video_pub,
            room_event_handle,
            cmd_input_sender,
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

impl Drop for TurboLivekitConnector {
    fn drop(&mut self) {
        if let Err(e) = futures::executor::block_on(self.shutdown()) {
            warn!("Error shutting down turbo webrtc | {e}");
        };
    }
}

unsafe impl Send for TurboLivekitConnector {}
unsafe impl Sync for TurboLivekitConnector {}
