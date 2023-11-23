use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use bytes::{BufMut, Bytes, BytesMut};
use async_trait::async_trait;
use ezsockets::{ClientConfig, RawMessage, SocketConfig, Client, MessageSignal, MessageStatus, client::ClientCloseMode, WSError, CloseFrame};
use deepgram::Deepgram;
use futures::StreamExt;
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use log::{error, info};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, Map, json};
use tokio::sync::mpsc::UnboundedSender;


#[derive(Clone)]
pub struct STT{
    ws_client: Client<WSClient>
}

struct WSClient {
    to_gpt: tokio::sync::mpsc::UnboundedSender<String>,
}

#[async_trait]
impl ezsockets::ClientExt for WSClient {
    type Call = ();

    async fn on_text(&mut self, text: String) -> Result<(), ezsockets::Error> {
        let data : Value = serde_json::from_str(&text)?;
        let transcript_details = data["channel"]["alternatives"][0].clone();
        
        info!("received message from deepgram: {transcript_details}");
        self.to_gpt.send(transcript_details.to_string())?;
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

    async fn on_connect_fail(&mut self, _error: WSError) -> Result<ClientCloseMode, ezsockets::Error> {
        info!("DEEPGRAM connection FAIL 💔");
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_close(&mut self, _frame: Option<CloseFrame>) -> Result<ClientCloseMode, ezsockets::Error> {
        info!("DEEPGRAM connection CLOSE 💔");
        Ok(ClientCloseMode::Reconnect)
    }

    async fn on_disconnect(&mut self) -> Result<ClientCloseMode, ezsockets::Error> {
        info!("DEEPGRAM disconnect 💔");
        Ok(ClientCloseMode::Reconnect)
    }
}

impl STT {
    pub const LATENCY_FRAMES: f32 = (Self::LATENCY_MS / 1_000.0) * Self::SAMPLE_RATE_F32;
    // Uses a delay of `LATENCY_MS` milliseconds in case the default input and output streams are not precisely synchronised
    pub const LATENCY_MS: f32 = 5000.0;
    pub const LATENCY_SAMPLES: u32 = Self::LATENCY_FRAMES as u32 * Self::NUM_OF_CHANNELS;
    pub const NUM_ITERS: usize = 2;
    pub const NUM_ITERS_SAVED: usize = 2;
    pub const NUM_OF_CHANNELS: u32 = 1;
    pub const SAMPLE_RATE: u32 = 1600;
    pub const SAMPLE_RATE_F32: f32 = Self::SAMPLE_RATE as f32;
    pub const SAMPLING_FREQ: f32 = Self::SAMPLE_RATE_F32 / 2.0;

    const MIN_AUDIO_MS_CHUNK: f32 = 20.0;

    pub async fn new(gpt_input_tx: tokio::sync::mpsc::UnboundedSender<String>) -> anyhow::Result<Self> {
        let deepgram_api_key = std::env::var("DEEPGRAM_API_KEY").expect("The DEEPGRAM_API_KEY env variable is required!");

        let config = ClientConfig::new("wss://api.deepgram.com/v1/listen").socket_config(SocketConfig {
            heartbeat: Duration::from_secs(8),
            timeout: Duration::from_secs(30 * 60), // 30 minutes
            heartbeat_ping_msg_fn: Arc::new(|_t: Duration| RawMessage::Text(r#"{ "type": "KeepAlive" }"#.into())),
        })
            .header("authorization", &format!("token {}", deepgram_api_key))
            .query_parameter("encoding", "linear16")
            .query_parameter("sample_rate", &Self::SAMPLE_RATE.to_string())
            .query_parameter("channels", &Self::NUM_OF_CHANNELS.to_string())
            .query_parameter("model", "2-conversationalai")
            .query_parameter("smart_format", "true")
            .query_parameter("filler_words", "true")
            .query_parameter("version", "latest")
            .query_parameter("tier", "nova")
        ;

        let (ws_client, future) = ezsockets::connect(|_client| 
            WSClient {to_gpt: gpt_input_tx}, 
            config
        ).await;

        Ok(Self { ws_client })
    }
    fn send(&self, bytes: impl Into<Vec<u8>>) -> anyhow::Result<MessageStatus> {
        let signal = self.ws_client.binary(bytes)?;
        Ok(signal.status())
    }

}

pub async fn transcribe(
    stt_client: STT,
    mut audio_stream: NativeAudioStream,
)  {
    // let mut curr_audio_len = 0.0_f32; // in ms
    
    while let Some(frame) = audio_stream.next().await {
        let num_of_samples = frame.data.len();
        // curr_audio_len += (num_of_sample as u32 / frame.sample_rate) as f32 /1000.0;
        // if curr_audio_len > STT::MIN_AUDIO_MS_CHUNK {
        //     curr_audio_len = 0.0;
        // }

        let mut bytes = BytesMut::with_capacity(num_of_samples * 2);
        frame.data.iter().for_each(|sample|bytes.put_i16_le(*sample));
        match stt_client.send(bytes.freeze()){
            Ok(status) => info!("Sent audio to deegram | Msg status {status:?}"),
            Err(e) => error!("Error sending audio bytes to deepgram ws {e}")
        };
    }
}

impl Drop for STT {
    fn drop(&mut self) {
        if let Err(e) = self.send([]){
            error!("Error shutting down STT  / Deepgram connection | Reason - {e}");
        };
    }
}