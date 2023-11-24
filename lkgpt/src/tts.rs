use anyhow::bail;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use deepgram::Deepgram;
use ezsockets::{
    client::ClientCloseMode, Client, ClientConfig, CloseFrame, MessageSignal, MessageStatus,
    RawMessage, SocketConfig, WSError,
};
use futures::StreamExt;
use livekit::webrtc::{
    audio_frame::AudioFrame, audio_source::native::NativeAudioSource,
    audio_stream::native::NativeAudioStream, native::audio_resampler,
};
use log::{error, info};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    env,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::stt::STT;

#[derive(Serialize)]
struct VoiceSettings {
    stability: f32,
    similarity_boost: bool,
}

#[derive(Serialize)]
struct BOSMessage<'a> {
    text: &'a str,
    voice_settings: VoiceSettings,
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

struct NormalizedAlignment {
    char_start_times_ms: Vec<u8>,
    chars_durations_ms: Vec<u8>,
    chars: Vec<char>,
}
struct ElevenLabs {
    audio: String,
    isFinal: bool,
    normalizedAlignment: NormalizedAlignment,
}

#[derive(Clone)]
pub struct TTS {
    ws_client: Option<Client<WSClient>>,
    pub started: bool,
    eleven_labs_api_key: String,
}

struct WSClient {
    audio_src: NativeAudioSource,
    tts_client_ref: Arc<Mutex<TTS>>,
}

fn vec_u8_to_vec_i16(input: Vec<u8>) -> Vec<i16> {
    // Ensure that the input Vec<u8> has an even number of elements
    if input.len() % 2 != 0 {
        panic!("Input Vec<u8> must have an even number of elements");
    }

    input
        .chunks(2)
        .map(|chunk| {
            // Convert each pair of u8 to one i16
            // Little-endian order: The first byte is the least significant
            i16::from_le_bytes([chunk[0], chunk[1]])
        })
        .collect()
}

#[async_trait]
impl ezsockets::ClientExt for WSClient {
    type Call = ();

    async fn on_text(&mut self, text: String) -> Result<(), ezsockets::Error> {
        let data: Value = serde_json::from_str(&text)?;
        let transcript_details = data["audio"].clone();

        if transcript_details != Value::Null {
            let data =
                general_purpose::STANDARD_NO_PAD.decode(transcript_details.as_str().unwrap())?;

            const FRAME_DURATION: Duration = Duration::from_millis(500); // Write 0.5s of audio at a time
            let ms = FRAME_DURATION.as_millis() as u32;

            let num_channels = self.audio_src.num_channels();
            let sample_rate = self.audio_src.sample_rate();
            let num_samples = (sample_rate / 1000 * ms) as usize;
            let samples_per_channel = num_samples as u32;

            // let mut resampler = audio_resampler::AudioResampler::default();
            // resampler.

            let audio_frame = AudioFrame {
                data: vec_u8_to_vec_i16(data).into(),
                num_channels,
                sample_rate,
                samples_per_channel,
            };

            self.audio_src.capture_frame(&audio_frame).await?;
        } else {
            error!("received message from eleven labs: {text}");
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
        info!("ELEVEN LABS CONNECTED 🎉");
        Ok(())
    }

    async fn on_connect_fail(
        &mut self,
        _error: WSError,
    ) -> Result<ClientCloseMode, ezsockets::Error> {
        info!("ELEVEN LABS connection FAIL 💔");
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_close(
        &mut self,
        _frame: Option<CloseFrame>,
    ) -> Result<ClientCloseMode, ezsockets::Error> {
        info!("ELEVEN LABS connection CLOSE 💔");
        let mut tts = self.tts_client_ref.lock();
        tts.started = false;
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_disconnect(&mut self) -> Result<ClientCloseMode, ezsockets::Error> {
        info!("ELEVEN LABS disconnect 💔");
        Ok(ClientCloseMode::Reconnect)
    }
}

impl TTS {
    pub fn new() -> anyhow::Result<Self> {
        let eleven_labs_api_key = std::env::var("ELEVENLABS_API_KEY")
            .expect("The ELEVENLABS_API_KEY env variable is required!");

        Ok(Self {
            ws_client: None,
            started: false,
            eleven_labs_api_key,
        })
    }

    pub async fn setup_ws_client(&mut self, audio_src: NativeAudioSource) -> anyhow::Result<()> {
        let ws_client = self.connect_ws_client(audio_src).await?;
        self.started = true;
        self.ws_client = Some(ws_client);
        Ok(())
    }

    async fn connect_ws_client(
        &mut self,
        audio_src: NativeAudioSource,
    ) -> anyhow::Result<Client<WSClient>> {
        let voice_id = "L1oawlP7wF6KPWjLuHcF";
        let model = "eleven_monolingual_v1";

        let url = url::Url::parse(&format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{voice_id}/stream-input?model_id={model}"
        ))
        .unwrap();

        let config = ClientConfig::new(url)
            .socket_config(SocketConfig {
                heartbeat: Duration::from_secs(10),
                timeout: Duration::from_secs(30 * 60), // 30 minutes
                heartbeat_ping_msg_fn: Arc::new(|_t: Duration| {
                    RawMessage::Text(
                        serde_json::to_string(&RegularMessage {
                            text: "  ".to_string(),
                            try_trigger_generation: true,
                        })
                        .unwrap(),
                    )
                }),
            })
            .header("xi-api-key", &self.eleven_labs_api_key)
            .header("Content-Type", "application/json")
            .header("optimize_streaming_latency", "3")
            .header("output_format", "pcm_16000");

        let (ws_client, future) = ezsockets::connect(
            |_client| WSClient {
                audio_src,
                tts_client_ref: Arc::new(Mutex::new(self.clone())),
            },
            config,
        )
        .await;

        ws_client.text(serde_json::to_string(&BOSMessage {
            text: " ",
            voice_settings: VoiceSettings {
                stability: 0.5,
                similarity_boost: false,
            },
        })?)?;
        Ok(ws_client)
    }

    pub fn start(&mut self) -> anyhow::Result<()> {
        self.started = true;
        self.send(" ".to_string())?;
        Ok(())
    }

    pub fn send(&mut self, msg: String) -> anyhow::Result<MessageStatus> {
        let msg = match msg.as_str() {
            "" => serde_json::to_string(&EOSMessage { text: "" }),
            " " => serde_json::to_string(&BOSMessage {
                text: " ",
                voice_settings: VoiceSettings {
                    stability: 0.5,
                    similarity_boost: false,
                },
            }),
            msg => serde_json::to_string(&RegularMessage {
                text: format!("{msg} "),
                try_trigger_generation: true,
            }),
        };
        let msg = msg?;

        if !self.started {
            self.start()?;
        }

        if self.ws_client.as_ref().is_none() {
            bail!("ws_client is none");
        }

        info!("sending to eleven labs {msg}");

        Ok(self.ws_client.as_ref().unwrap().text(msg)?.status())
    }
}

impl Drop for TTS {
    fn drop(&mut self) {
        info!("DROPPING TTS");
        if let Err(e) = self.send("".to_owned()) {
            error!("Error shutting down TTS  / Eleven Labs connection | Reason - {e}");
        };
    }
}
