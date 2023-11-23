use std::{
    sync::Arc,
    time::{Duration, Instant}, env,
};
use bytes::{BufMut, Bytes, BytesMut};
use async_trait::async_trait;
use ezsockets::{ClientConfig, RawMessage, SocketConfig, Client, MessageSignal, MessageStatus, client::ClientCloseMode, CloseFrame, WSError};
use deepgram::Deepgram;
use futures::StreamExt;
use livekit::webrtc::{audio_stream::native::NativeAudioStream, audio_source::native::NativeAudioSource};
use log::{error, info};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, Map, json};
use tokio::sync::mpsc::UnboundedSender;


#[derive(Serialize)]
struct VoiceSettings {
    stability: f32,
    similarity_boost: bool,
    xi_api_key: String, 
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
    try_trigger_generation: bool
}

struct NormalizedAlignment {
    char_start_times_ms: Vec<u8>,
    chars_durations_ms: Vec<u8>,
    chars: Vec<char>,
}
struct ElevenLabs{
    audio: String,
    isFinal: bool,
    normalizedAlignment: NormalizedAlignment
}

#[derive(Clone)]
pub struct TTS{
    ws_client: Option<Client<WSClient>>,
    pub started: bool,
    eleven_labs_api_key: String
}

struct WSClient {
    audio_src: NativeAudioSource,
    tts_client_ref: Arc<Mutex<TTS>>
}

#[derive(Debug, Serialize)]
struct PingMsg<'a> {
    r#type: &'a str
}

#[async_trait]
impl ezsockets::ClientExt for WSClient {
    type Call = ();

    async fn on_text(&mut self, text: String) -> Result<(), ezsockets::Error> {
        // {"message":"Could not parse input message as JSON, ensure to send a valid JSON.","error":"input_message_json_decode_fail","code":1008}
        info!("received message from eleven labs: {text}");
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

    async fn on_connect_fail(&mut self, _error: WSError) -> Result<ClientCloseMode, ezsockets::Error> {
        info!("ELEVEN LABS connection FAIL 💔");
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_close(&mut self, _frame: Option<CloseFrame>) -> Result<ClientCloseMode, ezsockets::Error> {
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
        let eleven_labs_api_key = std::env::var("ELEVENLABS_API_KEY").expect("The ELEVENLABS_API_KEY env variable is required!");

        Ok(Self { ws_client:None, started: false, eleven_labs_api_key })
    }


    pub async fn setup_ws_client(&mut self, audio_src: NativeAudioSource) -> anyhow::Result<()> {
        let ws_client = self.connect_ws_client(audio_src).await?;
        self.ws_client = Some(ws_client);
        Ok(())
    }
    
    async fn connect_ws_client(&mut self, audio_src: NativeAudioSource) -> anyhow::Result<Client<WSClient>> {    
        let voice_id = "L1oawlP7wF6KPWjLuHcF";
        let model = "eleven_monolingual_v2";
        
        let url = url::Url::parse(&format!("wss://api.elevenlabs.io/v1/text-to-speech/{voice_id}/stream-input?model_id={model}")).unwrap();

        let config = ClientConfig::new(url).socket_config(SocketConfig {
            heartbeat: Duration::from_secs(8),
            timeout: Duration::from_secs(30 * 60), // 30 minutes
            ..Default::default()
            // heartbeat_ping_msg_fn: Arc::new(|_t: Duration| {
            //     let x = RawMessage::Text(serde_json::to_string(&PingMsg{r#type:"KeepAlive"}).unwrap());
            //     info!("ping message {x:?}");
            //     x                
            // }),
        })
        .header("xi-api-key", &self.eleven_labs_api_key)
        .header("Content-Type", "application/json")
        ;

        let (ws_client, future) = ezsockets::connect(
            |_client| WSClient {
                audio_src,
                tts_client_ref: Arc::new(Mutex::new(self.clone())),
            },
            config,
        ).await;
        Ok(ws_client)
    }


    pub fn start(&mut self) -> anyhow::Result<()> {
        self.started = true;
        self.send(" ".to_string())?;
        Ok(())
    }

    pub fn send(&mut self, msg: String) -> anyhow::Result<MessageStatus> {
        let msg = match msg.as_str() {
            "" => serde_json::to_string(&EOSMessage{text:""}),
            " " => serde_json::to_string(&BOSMessage{text: " ",
                                            voice_settings: VoiceSettings {
                                                        stability: 0.5,
                                                        similarity_boost: false,
                                                        xi_api_key: self.eleven_labs_api_key.clone(), 
                                            },
            }),
            msg => serde_json::to_string(&RegularMessage{text:format!("{msg} "), try_trigger_generation:true}),
        };
        let msg = msg?;

        if !self.started {
            self.start()?;
        }
        info!("sending to eleven labs {msg}");
        Ok(self.ws_client.as_ref().unwrap().text(msg)?.status())
    }

}



impl Drop for TTS {
    fn drop(&mut self) {
        info!("DROPPING TTS");
        if let Err(e) = self.send("".to_owned()){
            error!("Error shutting down TTS  / Eleven Labs connection | Reason - {e}");
        };
    }
}