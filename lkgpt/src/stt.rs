use std::time::Duration;

use bevy::ecs::{
    system::{Res, ResMut, Resource},
    world::{FromWorld, World},
};
use deepgram::{
    transcription::live::{
        options::{Model, Options},
        response::Response,
        DeepgramLive,
    },
    Deepgram,
};
use futures::{stream::SplitSink, SinkExt, StreamExt};
use rodio::cpal::Sample;

use crate::{AsyncRuntime, DEEPGRAM_API_KEY};

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
    last_msg_time: std::time::Instant,
    tx: SplitSink<DeepgramLive, Vec<u8>>,
    rx: crossbeam_channel::Receiver<String>,
}

impl STT {
    pub const NUM_OF_CHANNELS: u32 = 1;
    pub const SAMPLE_RATE: u32 = 44100;
}

impl FromWorld for STT {
    fn from_world(world: &mut World) -> Self {
        let async_rt = world.get_resource::<AsyncRuntime>().unwrap();

        let ws = match async_rt.rt.block_on(async { connect_to_deepgram().await }) {
            Ok(ws) => ws,
            Err(e) => panic!("Failed to connect to Deepgram: {}", e),
        };

        let last_msg_time = std::time::Instant::now();

        let (ws_tx, mut socket_stream) = ws.split();

        let (tx, rx) = crossbeam_channel::unbounded();

        async_rt.rt.spawn(async move {
            while let Some(res) = socket_stream.next().await {
                match res {
                    Ok(resp) => match resp {
                        Response::Results(result) => {
                            log::info!("😈 DEEPGRAM RESULT : {:?}", result);
                            let txt = result.channel.alternatives[0].transcript.clone();
                            if let Err(e) = tx.send(txt) {
                                log::error!("Failed to send transcribed text: {}", e);
                            };
                        },
                        Response::Metadata(metadata) => {
                            log::info!("STT metadata from deepgram : {:?}", metadata);
                        },
                    },
                    Err(e) => {
                        log::error!("Deepgram websocket stream error: {:#?}", e);
                    },
                }
            }
            log::error!("Deepgram websocket stream ended");
        });

        log::info!("Connected to Deepgram");
        Self { tx: ws_tx, rx, last_msg_time }
    }
}

async fn connect_to_deepgram() -> anyhow::Result<DeepgramLive> {
    let deepgram_api_key = std::env::var(DEEPGRAM_API_KEY).unwrap();

    let dg_client = Deepgram::new(&deepgram_api_key)?;
    let options = Options::builder()
        .model(Model::CustomId("nova-2".to_string()))
        .punctuate(true)
        .version("latest")
        .build();

    // ("smart_format", "true"),
    // ("filler_words", "true"),

    let deepgram_socket = dg_client.transcription().live(&options).await?;

    Ok(deepgram_socket)
}

// SYSTEMS
pub fn receive_audio_input(
    audio_channel: Res<AudioInputChannel>,
    mut stt_websocket: ResMut<STT>,
    async_runtime: Res<AsyncRuntime>,
) {
    while let Ok(audio_buffer) = audio_channel.rx.try_recv() {
        log::info!("Received audio buffer from mic");
        let sample_bytes =
            audio_buffer.into_iter().map(|sample| sample.to_sample::<u8>()).collect::<Vec<u8>>();

        match async_runtime.rt.block_on(async { stt_websocket.tx.send(sample_bytes).await }) {
            Ok(_) => {
                log::info!("Sent audio buffer to Deepgram");
                stt_websocket.last_msg_time = std::time::Instant::now();
            },
            Err(e) => {
                log::error!("Failed to send audio buffer to Deepgram: {e:?}");
            },
        };
    }

    // Send PING to Deepgram if no audio input for 10 seconds | Prevents Deepgram from closing the websocket connection
    if stt_websocket.last_msg_time.elapsed() > Duration::from_secs(10) {
        log::warn!("No audio input for 10 seconds, sending PING to Deepgram");
        match async_runtime.rt.block_on(async { stt_websocket.tx.send(vec![0_u8; 250]).await }) {
            Ok(_) => {
                log::info!("Sent PING audio buffer to Deepgram");
                stt_websocket.last_msg_time = std::time::Instant::now();
            },
            Err(e) => {
                log::error!("Failed to send PING audio buffer to Deepgram: {e:?}");
            },
        };
    }
}

pub fn send_transcribed_audio_to_llm(
    llm_channel: Res<crate::llm::LLMChannel>,
    stt_websocket: ResMut<STT>,
) {
    while let Ok(message) = stt_websocket.rx.try_recv() {
        log::info!("\n\n\n\n\n\n\n\n\n\ntranscribed text: {}", message);
        if let Err(e) = llm_channel.tx.send(message) {
            log::error!("Failed to send transcribed text: {}", e);
        };
    }
}
