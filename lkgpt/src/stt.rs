use async_trait::async_trait;
use bevy::ecs::system::Resource;
use bytes::{BufMut, Bytes, BytesMut};
use deepgram::Deepgram;
use ezsockets::{
    client::ClientCloseMode, Client, ClientConfig, CloseFrame, MessageSignal, MessageStatus,
    RawMessage, SocketConfig, WSError,
};
use futures::StreamExt;
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use log::{error, info};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

use crate::{AsyncRuntime, DEEPGRAM_API_KEY};

#[derive(Clone, Resource)]
pub struct STT {
    ws_client: Arc<Client<WSClient>>,
}

struct WSClient {
    llm_channel_tx: crossbeam_channel::Sender<String>,
}

#[async_trait]
impl ezsockets::ClientExt for WSClient {
    type Call = ();

    async fn on_text(&mut self, text: String) -> Result<(), ezsockets::Error> {
        let data: Value = serde_json::from_str(&text)?;
        let transcript_details = data["channel"]["alternatives"][0].clone();

        info!("\n\n\nreceived message from deepgram: {}", data);
        info!("\n\n\nreceived message from deepgram: {}", transcript_details);

        // if transcript_details!= Value::Null {
        //     self.to_gpt.send(transcript_details.to_string())?;
        // }

        Ok(())
    }

    async fn on_binary(&mut self, bytes: Vec<u8>) -> Result<(), ezsockets::Error> {
        info!("received bytes: {bytes:?}");
        Ok(())
    }

    async fn on_call(&mut self, call: Self::Call) -> Result<(), ezsockets::Error> {
        info!("DEEPGRAM ON CALL: {call:?}");
        let () = call;
        Ok(())
    }

    async fn on_connect(&mut self) -> Result<(), ezsockets::Error> {
        info!("DEEPGRAM CONNECTED 🎉");
        Ok(())
    }

    async fn on_connect_fail(&mut self, e: WSError) -> Result<ClientCloseMode, ezsockets::Error> {
        error!("DEEPGRAM connection FAIL 💔 {e}");
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_close(
        &mut self,
        frame: Option<CloseFrame>,
    ) -> Result<ClientCloseMode, ezsockets::Error> {
        error!("DEEPGRAM connection CLOSE 💔 {frame:?}");
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_disconnect(&mut self) -> Result<ClientCloseMode, ezsockets::Error> {
        error!("DEEPGRAM disconnect 💔");
        Ok(ClientCloseMode::Reconnect)
    }
}

impl STT {
    pub async fn new(llm_channel_tx: crossbeam_channel::Sender<String>) -> anyhow::Result<Self> {
        let deepgram_api_key = std::env::var(DEEPGRAM_API_KEY).unwrap();

        let config = ClientConfig::new("wss://api.deepgram.com/v1/listen")
            .socket_config(SocketConfig {
                heartbeat: Duration::from_secs(11),
                timeout: Duration::from_secs(30 * 60), // 30 minutes
                heartbeat_ping_msg_fn: Arc::new(|_t: Duration| {
                    RawMessage::Text(json!({
                        "type": "KeepAlive",
                    }).to_string())
                }),
            })
            .header("Authorization", &format!("Token {}", deepgram_api_key))
            .query_parameter("model", "enhanced")
            // .query_parameter("model", "nova-2-conversationalai")
            .query_parameter("smart_format", "true")
            .query_parameter("filler_words", "true");

        let (ws_client, _) =
            ezsockets::connect(|_client| WSClient { llm_channel_tx }, config).await;

        Ok(Self { ws_client: Arc::new(ws_client) })
    }

    pub fn send(&self, bytes: impl Into<Vec<u8>>) -> anyhow::Result<MessageStatus> {
        let signal = self.ws_client.binary(bytes)?;
        Ok(signal.status())
    }
}
