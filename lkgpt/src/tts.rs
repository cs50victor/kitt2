use anyhow::bail;
use async_trait::async_trait;
use base64::{
    engine::general_purpose::{self},
    Engine,
};
use bevy::ecs::system::Resource;

use ezsockets::{
    client::ClientCloseMode, Client, ClientConfig, CloseFrame, MessageStatus, RawMessage,
    SocketConfig, WSError,
};
use futures::StreamExt;
use livekit::webrtc::{audio_frame::AudioFrame, audio_source::native::NativeAudioSource};
use log::{error, info};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use std::io::Cursor;

use crate::ELEVENLABS_API_KEY;

#[derive(Serialize)]
struct VoiceSettings {
    stability: f32,
    similarity_boost: bool,
}

#[derive(Serialize)]
struct GenerationConfig {
    chunk_length_schedule: [u8; 1],
}

#[derive(Serialize)]
struct BOSMessage<'a> {
    text: &'a str,
    try_trigger_generation: bool,
    voice_settings: VoiceSettings,
    generation_config: GenerationConfig,
}

#[derive(Serialize)]
struct EOSMessage<'a> {
    text: &'a str,
}

#[derive(Serialize)]
struct RegularMessage {
    text: String,
    try_trigger_generation: bool,
}

#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Resource)]
pub struct TTS {
    ws_client: Client<WSClient>,
    pub started: Arc<AtomicBool>,
    eleven_labs_api_key: String,
}

impl TTS {
    pub const NUM_OF_CHANNELS: u32 = 1;
    pub const SAMPLE_RATE: u32 = 44100;
}

struct WSClient {
    audio_src: NativeAudioSource,
    tts_ws_started: Arc<AtomicBool>,
}

fn decode_base64_audio(base64_audio: &str) -> anyhow::Result<Vec<i16>> {
    let data = general_purpose::STANDARD.decode(base64_audio)?;
    let decoder = rodio::Decoder::new(Cursor::new(data))?;

    Ok(decoder.into_iter().collect::<Vec<i16>>())
}

#[async_trait]
impl ezsockets::ClientExt for WSClient {
    type Call = ();

    async fn on_text(&mut self, text: String) -> Result<(), ezsockets::Error> {
        // info!("raw message from eleven labs {text:?}");
        let data: Value = serde_json::from_str(&text)?;
        let base64_audio = data["audio"].clone();

        info!("incoming speech from eleven labs");
        if base64_audio != Value::Null {
            let data = std::borrow::Cow::from(decode_base64_audio(base64_audio.as_str().unwrap())?);

            const FRAME_DURATION: Duration = Duration::from_millis(500); // Write 0.5s of audio at a time
            let ms = FRAME_DURATION.as_millis() as u32;

            let num_channels = self.audio_src.num_channels();
            let sample_rate = self.audio_src.sample_rate();
            let samples_per_channel = 1_u32;

            let num_samples = (sample_rate / 1000 * ms) as usize;

            let audio_frame = AudioFrame { data, num_channels, sample_rate, samples_per_channel };

            self.audio_src.capture_frame(&audio_frame).await?;
        } else {
            error!("received null audio from eleven labs: {text:?}");
        }

        Ok(())
    }

    async fn on_binary(&mut self, bytes: Vec<u8>) -> Result<(), ezsockets::Error> {
        info!("received bytes: {bytes:?}");
        Ok(())
    }

    async fn on_call(&mut self, call: Self::Call) -> Result<(), ezsockets::Error> {
        info!("ELEVEN LABS WTF");
        let () = call;
        Ok(())
    }

    async fn on_connect(&mut self) -> Result<(), ezsockets::Error> {
        self.tts_ws_started.store(true, Ordering::Relaxed);
        info!("ELEVEN LABS CONNECTED 🎉");
        Ok(())
    }

    async fn on_connect_fail(
        &mut self,
        _error: WSError,
    ) -> Result<ClientCloseMode, ezsockets::Error> {
        error!("ELEVEN LABS connection FAIL");
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_close(
        &mut self,
        _frame: Option<CloseFrame>,
    ) -> Result<ClientCloseMode, ezsockets::Error> {
        error!("ELEVEN LABS connection CLOSE");
        self.tts_ws_started.store(false, Ordering::Relaxed);
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_disconnect(&mut self) -> Result<ClientCloseMode, ezsockets::Error> {
        error!("ELEVEN LABS disconnected");
        Ok(ClientCloseMode::Reconnect)
    }
}

impl TTS {
    pub async fn new(audio_src: NativeAudioSource) -> anyhow::Result<Self> {
        let eleven_labs_api_key = std::env::var(ELEVENLABS_API_KEY).unwrap();
        let started = Arc::new(AtomicBool::new(true));

        let voice_id = "21m00Tcm4TlvDq8ikWAM";
        let model = "eleven_turbo_v2";

        let url = url::Url::parse_with_params(
            &format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{voice_id}/stream-input?model_id={model}"
        ),
            &[("optimize_streaming_latency", "3")],
        )
        .unwrap();

        let config = ClientConfig::new(url)
            .socket_config(SocketConfig {
                heartbeat: Duration::from_secs(10),
                timeout: Duration::from_secs(30 * 60), // 30 minutes
                heartbeat_ping_msg_fn: Arc::new(|_t: Duration| {
                    RawMessage::Text(
                        serde_json::to_string(&RegularMessage {
                            text: "  ".to_string(),
                            try_trigger_generation: false,
                        })
                        .unwrap(),
                    )
                }),
            })
            .header("xi-api-key", eleven_labs_api_key.clone());

        let (ws_client, _) = ezsockets::connect(
            |_client| WSClient { audio_src, tts_ws_started: started.clone() },
            config,
        )
        .await;

        ws_client.text(serde_json::to_string(&BOSMessage {
            text: " ",
            try_trigger_generation: true,
            voice_settings: VoiceSettings { stability: 0.8, similarity_boost: true },
            generation_config: GenerationConfig { chunk_length_schedule: [50] },
        })?)?;

        Ok(Self { ws_client, started, eleven_labs_api_key })
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        self.started.store(true, Ordering::Relaxed);
        self.send(" ".to_string())?;
        Ok(())
    }

    pub fn send(&mut self, msg: String) -> anyhow::Result<MessageStatus> {
        let msg = match msg.as_str() {
            "" => serde_json::to_string(&EOSMessage { text: "" }),
            " " => serde_json::to_string(&BOSMessage {
                text: " ",
                try_trigger_generation: true,
                voice_settings: VoiceSettings { stability: 0.8, similarity_boost: true },
                generation_config: GenerationConfig { chunk_length_schedule: [50] },
            }),
            msg => serde_json::to_string(&RegularMessage {
                text: format!("{msg} "),
                try_trigger_generation: true,
            }),
        };
        let msg = msg?;

        if !self.started.load(Ordering::Relaxed) {
            self.start()?;
        }

        info!("sending to eleven labs {msg}");

        Ok(self.ws_client.text(msg)?.status())
    }
}
