use futures::{SinkExt, StreamExt};
use livekit::webrtc::audio_source::native::NativeAudioSource;
use log::{error, info, warn};
use parking_lot::Mutex;
use serde::Serialize;
use std::{f32, sync::Arc};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

#[derive(Clone)]
pub struct TTS {
    socket: Arc<Mutex<WebSocketStream<MaybeTlsStream<TcpStream>>>>,
    eleven_labs_url: String,
    eleven_api_key: String,
}

#[derive(Serialize)]
struct TTSMsg<'a> {
    text: &'a str,
    try_trigger_generation: bool,
}

#[derive(Serialize)]
struct VoiceSettings {
    stability: f32,
    similarity_boost: bool,
}

// bos_message | Begin of Service Message
#[derive(Serialize)]
struct BOSMessage<'a> {
    text: &'a str,
    voice_settings: VoiceSettings,
    xi_api_key: String,
}

impl TTS {
    pub async fn new() -> anyhow::Result<Self> {
        let eleven_api_key = std::env::var("ELEVENLABS_API_KEY")
            .expect("ELEVENLABS_API_KEY must be use text to speech");
        let voice_id = "21m00Tcm4TlvDq8ikWAM";
        let model = "eleven_multilingual_v2";
        let eleven_labs_url = format!(
            "wss://api.elevenlabs.io/v1/text-to-speech/{voice_id}/stream-input?model_id={model}"
        );
        info!("{eleven_labs_url}");

        let (mut socket, _) = connect_async(eleven_labs_url.clone()).await?;

        let bos_msg = serde_json::to_string(&BOSMessage {
            text: " ",
            voice_settings: VoiceSettings { stability: 0.5, similarity_boost: true },
            xi_api_key: eleven_api_key.clone(),
        })?;

        // start service
        socket.send(Message::Text(bos_msg)).await?;

        let msg = serde_json::to_string(&TTSMsg {
            text: &("These challenges that you face are going to do their best to take you down "),
            try_trigger_generation: true,
        })?;
        socket.send(Message::Text(msg)).await?;

        Ok(Self { socket: Arc::new(Mutex::new(socket)), eleven_labs_url, eleven_api_key })
    }

    pub async fn send(&mut self, msg: String) -> anyhow::Result<()> {
        let mut socket = self.socket.lock();
        let msg =
            serde_json::to_string(&TTSMsg { text: &(msg + " "), try_trigger_generation: true })?;
        socket.send(Message::Text(msg)).await?;
        Ok(())
    }

    pub async fn handle_voice_stream(&mut self, lsdk_audio_src: NativeAudioSource) {
        let mut socket = self.socket.lock();
        while let Some(voice_base64) = socket.next().await {
            match voice_base64 {
                Ok(voice_base64) => match voice_base64 {
                    Message::Text(audio_base64) => {
                        info!("\n\n\n\nconvert this base64 voice stream later | {audio_base64:#?}")
                    },
                    Message::Close(_) => {
                        let mut self_clone = self.clone();
                        if let Err(e) = self_clone.restart_ws_connection().await {
                            error!("Coudln't restart ws connection to eleven labs {e}");
                        }
                    },
                    _ => {},
                },
                Err(e) => {
                    error!("\n\n\n\nvoice stream from api err {e}");
                },
            }
        }
    }

    async fn restart_ws_connection(&mut self) -> anyhow::Result<()> {
        let (mut socket, _) = connect_async(self.eleven_labs_url.clone()).await?;

        let bos_msg = serde_json::to_string(&BOSMessage {
            text: " ",
            voice_settings: VoiceSettings { stability: 0.5, similarity_boost: true },
            xi_api_key: self.eleven_api_key.clone(),
        })?;

        // start service
        socket.send(Message::Text(bos_msg)).await?;
        Ok(())
    }
}

#[derive(Serialize)]
struct EOSMessage<'a> {
    text: &'a str,
}

impl Drop for TTS {
    fn drop(&mut self) {
        let mut socket = self.socket.lock();
        let eos_msg = serde_json::to_string(&EOSMessage { text: "" }).unwrap();

        if let Err(e) = futures::executor::block_on(socket.send(Message::Text(eos_msg))) {
            warn!("couldn't send EOS message to eleven labs {e}");
        };

        if let Err(e) = futures::executor::block_on(socket.close(None)) {
            warn!("web socket connection close err {e}");
        };
    }
}
