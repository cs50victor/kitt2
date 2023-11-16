use std::sync::Arc;

use anyhow::Result;
use async_openai::{config::OpenAIConfig, Client as OPENAI_CLIENT};
use livekit::{publication::LocalTrackPublication, Room};

use log::{error, info, warn};
use parking_lot::Mutex;
use tokio::{
    sync::mpsc::{Receiver, UnboundedReceiver},
    task::JoinHandle,
};

use crate::{
    gpt::gpt, handle_room_events, publish_tracks, stt::STT, tts::TTS, TracksPublicationData, Turbo,
};

pub struct TurboLivekitConnector {
    room: Arc<Room>,
    text_input_tx: tokio::sync::mpsc::UnboundedSender<String>,
    cmd_input_sender: std::sync::mpsc::Sender<String>,
    room_event_handle: JoinHandle<Result<()>>,
    video_pub: LocalTrackPublication,
    audio_pub: LocalTrackPublication,
    gpt_thread_handle: JoinHandle<()>,
    tts_thread_handle: JoinHandle<()>,
    tts_receiver_handle: JoinHandle<()>,
    render_thread_handle: Option<JoinHandle<()>>,
}

impl TurboLivekitConnector {
    pub async fn new(
        room: Arc<Room>,
        room_events: tokio::sync::mpsc::UnboundedReceiver<livekit::RoomEvent>,
    ) -> Result<Self> {
        let (cmd_input_sender, cmd_input_receiver) = std::sync::mpsc::channel::<String>();
        let (text_input_tx, main_input_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (to_voice_tx, from_gpt_rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        let open_ai_org_id = std::env::var("OPENAI_ORG_ID").expect("OPENAI_ORG_ID must be");

        let openai_client =
            OPENAI_CLIENT::with_config(OpenAIConfig::new().with_org_id(open_ai_org_id));
        // let stt_cleint = Arc::new(STT::new()?);

        let mut turbo = Turbo::new()?.load_basic_scene()?;
        let mut tts_client = TTS::new().await?;

        let TracksPublicationData { video_pub, video_src, audio_src, audio_pub } =
            publish_tracks(room.clone()).await?;

        let room_event_handle =
            tokio::spawn(handle_room_events(text_input_tx.clone(), room_events));
        // stt_cleint,

        let tts_client_for_gpt = tts_client.clone();
        let mut tts_client_for_receiver = tts_client.clone();
        let mut tts_client_x = tts_client.clone();

        let tts_receiver_handle = tokio::spawn(demo(from_gpt_rx, tts_client_for_receiver));

        // let tts_thread_handle = tokio::spawn(tts.transcribe(main_input_rx));

        let gpt_thread_handle = tokio::spawn(async {
            if let Err(e) = gpt(main_input_rx, openai_client, tts_client_for_gpt, to_voice_tx).await
            {
                error!("GPT thread exited with error: {e}");
            }
        });

        let tts_thread_handle =
            tokio::spawn(async move { tts_client_x.handle_voice_stream(audio_src).await });

        let render_thread_handle = tokio::spawn(async move {
            if let Err(e) = turbo.render(video_src).await {
                error!("Turbo graphics render thread exited with error: {e}");
            }
        });

        Ok(Self {
            room,
            text_input_tx,
            audio_pub,
            video_pub,
            room_event_handle,
            cmd_input_sender,
            gpt_thread_handle,
            tts_thread_handle,
            tts_receiver_handle,
            render_thread_handle: Some(render_thread_handle),
        })
    }

    pub fn get_thread_handle(&mut self) -> JoinHandle<()> {
        self.render_thread_handle.take().expect("render thread handle should not be None")
    }

    pub fn get_txt_input_sender(&mut self) -> tokio::sync::mpsc::UnboundedSender<String> {
        self.text_input_tx.clone()
    }

    async fn shutdown(&mut self) {
        match self.room.close().await {
            Ok(d) => {
                info!("Successfull closed room. {d:?}");
            },
            Err(e) => {
                warn!("Couldn't close livekit room. {e:?}");
            },
        };
    }
}

async fn demo(mut from_gpt_rx: UnboundedReceiver<String>, mut tts_client_for_receiver: TTS) {
    while let Some(text_chunk) = from_gpt_rx.recv().await {
        info!("text_chunk for TEXT TO VOICE [{text_chunk}]");
        if let Err(e) = tts_client_for_receiver.send(text_chunk.clone()).await {
            error!("Coudln't send text to text-to-speech channel - {e:?}");
        } else {
            info!("GPT SENT THIS TO VOICE LABS - {text_chunk}");
        }
    }
}

impl Drop for TurboLivekitConnector {
    fn drop(&mut self) {
        futures::executor::block_on(self.shutdown());
        warn!("DROPPED - TURBOWEBRTC CONNECTOR ");
    }
}

unsafe impl Send for TurboLivekitConnector {}
unsafe impl Sync for TurboLivekitConnector {}
