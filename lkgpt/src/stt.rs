use core::slice::SlicePattern;

use bevy::ecs::{
    system::{Res, ResMut, Resource},
    world::{FromWorld, World},
};
use deepgram::{
    transcription::live::{
        options::{Encoding, Model, Options},
        response::Response,
        DeepgramLive,
    },
    Deepgram,
};
use futures::{
    stream::{FusedStream, SplitSink},
    Sink, SinkExt, Stream, StreamExt,
};
use tokio_tungstenite::tungstenite::{self, http};
use url::Url;

use crate::{AsyncRuntime, DEEPGRAM_API_KEY_ENV};

#[derive(Resource)]
pub struct AudioInputChannel {
    pub tx: crossbeam_channel::Sender<Vec<i16>>,
    rx: crossbeam_channel::Receiver<Vec<i16>>,
}

impl FromWorld for AudioInputChannel {
    fn from_world(_: &mut World) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<i16>>();
        Self { tx, rx }
    }
}

/// Live Speech To Text using Deepgram's Websocket API
#[allow(clippy::upper_case_acronyms)]
#[derive(Resource)]
pub struct STT {
    tx: SplitSink<DeepgramLive, Vec<u8>>,
    rx: crossbeam_channel::Receiver<String>,
}

impl STT {
    pub const LATENCY_FRAMES: f32 = (Self::LATENCY_MS / 1_000.0) * Self::SAMPLE_RATE_F32;
    // Uses a delay of `LATENCY_MS` milliseconds in case the default input and output streams are not precisely synchronised
    pub const LATENCY_MS: f32 = 5000.0;
    pub const LATENCY_SAMPLES: u32 = Self::LATENCY_FRAMES as u32 * Self::NUM_OF_CHANNELS;
    const MIN_AUDIO_MS_CHUNK: u64 = 25;
    pub const NUM_ITERS: usize = 2;
    pub const NUM_ITERS_SAVED: usize = 2;
    pub const NUM_OF_CHANNELS: u32 = 1;
    pub const SAMPLE_RATE: u32 = 44100;
    //1600
    pub const SAMPLE_RATE_F32: f32 = Self::SAMPLE_RATE as f32;
    pub const SAMPLING_FREQ: f32 = Self::SAMPLE_RATE_F32 / 2.0;
}

impl FromWorld for STT {
    fn from_world(world: &mut World) -> Self {
        let async_rt = world.get_resource::<AsyncRuntime>().unwrap();

        let ws = match async_rt.rt.block_on(async { connect_to_deepgram().await }) {
            Ok(ws) => ws,
            Err(e) => panic!("Failed to connect to Deepgram: {}", e),
        };

        let (ws_tx, mut socket_stream) = ws.split();

        // let k = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>
        let (tx, rx) = crossbeam_channel::unbounded();

        async_rt.rt.spawn(async move {
            while let Some(Ok(resp)) = socket_stream.next().await {
                match resp {
                    Response::Results(result) => {
                        let txt = result.channel.alternatives[0].transcript.clone();
                        if let Err(e) = tx.send(txt) {
                            log::error!("Failed to send transcribed text: {}", e);
                        };
                    },
                    Response::Metadata(metadata) => {
                        log::info!("STT metadata from deepgram : {:?}", metadata);
                    },
                }
            }
        });

        log::info!("Connected to Deepgram");
        Self { tx: ws_tx, rx }
    }
}

async fn connect_to_deepgram() -> anyhow::Result<DeepgramLive> {
    let deepgram_api_key = std::env::var(DEEPGRAM_API_KEY_ENV).unwrap();

    let dg_client = Deepgram::new(&deepgram_api_key)?;
    let options = Options::builder()
        .model(Model::CustomId("nova-2-conversationalai".to_string()))
        .encoding_with_channels(
            Encoding::Linear16,
            STT::SAMPLE_RATE as usize,
            STT::NUM_OF_CHANNELS as usize,
        )
        .punctuate(true)
        .version("latest")
        .build();

    // ("smart_format", "true"),
    // ("filler_words", "true"),

    let deepgram_socket = dg_client.transcription().live(&options).await?;

    Ok(deepgram_socket)
}

// SYSTEM
pub fn receive_and_process_audio(
    audio_channel: Res<AudioInputChannel>,
    llm_channel: Res<crate::llm::LLMChannel>,
    mut stt_websocket: ResMut<STT>,
    async_runtime: Res<AsyncRuntime>,
) {
    while let Ok(audio_buffer) = audio_channel.rx.try_recv() {
        log::info!("receiving audio");
        let sample_bytes =
            audio_buffer.into_iter().flat_map(|sample| sample.to_le_bytes()).collect::<Vec<u8>>();

        let _ = async_runtime.rt.block_on(async { stt_websocket.tx.send(sample_bytes).await });
    }

    while let Ok(message) = stt_websocket.rx.try_recv() {
        log::info!("transcribed text: {}", message);
        llm_channel.tx.send(message);
    }
}
