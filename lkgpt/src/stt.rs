use bevy::ecs::{
    system::{Res, ResMut, Resource},
    world::{FromWorld, World},
};
use futures::{stream::SplitSink, SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{self, http};
use url::Url;

use crate::{AsyncRuntime, DEEPGRAM_API_KEY_ENV};

#[derive(Resource)]
pub struct AudioChannel {
    pub tx: crossbeam_channel::Sender<Vec<i16>>,
    rx: crossbeam_channel::Receiver<Vec<i16>>,
}

impl FromWorld for AudioChannel {
    fn from_world(_: &mut World) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<Vec<i16>>();
        Self { tx, rx }
    }
}

/// Live Speech To Text using Deepgram's Websocket API
#[allow(clippy::upper_case_acronyms)]
#[derive(Resource)]
pub struct STT {
    tx: SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tungstenite::Message,
    >,
    rx: crossbeam_channel::Receiver<tungstenite::Message>,
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
        let rt = world.get_resource::<AsyncRuntime>().unwrap();
        let rt = rt.rt.clone();

        let ws = rt.block_on(async { connect_to_deepgram().await });

        let (tx, rx) = crossbeam_channel::unbounded();

        let (ws_tx, mut ws_rx) = ws.split();

        // Here we spawn an indefinite async task which receives websocket messages from Deepgram and pipes
        // them into a crossbeam channel, allowing the main synchronous Bevy runtime to access them when
        // needed (e.g. once per frame in the game loop).
        rt.spawn(async move {
            while let Some(Ok(message)) = ws_rx.next().await {
                let _ = tx.send(message);
            }
        });

        Self { tx: ws_tx, rx }
    }
}

async fn connect_to_deepgram(
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let deepgram_api_key = std::env::var(DEEPGRAM_API_KEY_ENV).unwrap();

    let mut api = Url::parse("wss://api.deepgram.com/v1/listen").unwrap();
    api.query_pairs_mut().extend_pairs([
        ("encoding", "linear16"),
        ("sample_rate", &STT::SAMPLE_RATE.to_string()),
        ("channels", &STT::NUM_OF_CHANNELS.to_string()),
        ("model", "2-conversationalai"),
        ("smart_format", "true"),
        ("filler_words", "true"),
        ("version", "latest"),
        ("tier", "nova"),
    ]);

    let request = http::Request::builder()
        .method(http::Method::GET)
        .uri(api.as_str())
        .header("authorization", &format!("token {deepgram_api_key}"))
        .body(())
        .expect("Failed to build a connection request to Deepgram.");

    let (deepgram_socket, _) =
        tokio_tungstenite::connect_async(request).await.expect("Failed to connect to Deepgram.");

    deepgram_socket
}

// SYSTEM
pub fn receive_and_process_audio(
    audio_channel: Res<AudioChannel>,
    llm_channel: Res<crate::llm::LLMChannel>,
    mut stt_websocket: ResMut<STT>,
    async_runtime: Res<AsyncRuntime>,
) {
    while let Ok(audio_buffer) = audio_channel.rx.try_recv() {
        let sample_bytes =
            audio_buffer.into_iter().flat_map(|sample| sample.to_le_bytes()).collect();

        let rt = async_runtime.rt.clone();

        let _ = rt.block_on(async {
            stt_websocket.tx.send(tungstenite::Message::Binary(sample_bytes)).await
        });
    }

    while let Ok(message) = stt_websocket.rx.try_recv() {
        if let tungstenite::Message::Text(message) = message {
            log::info!("transcribed text: {}", message);
            llm_channel.tx.send(message);
        }
    }
}

/// A helper function for converting f32 PCM samples to i16 (linear16) samples.
/// Deepgram currently does not support f32 PCM.
fn f32_to_i16(sample: f32) -> i16 {
    let sample = sample * 32768.0;

    // This is a saturating cast. For more details, see:
    // <https://doc.rust-lang.org/reference/expressions/operator-expr.html#numeric-cast>.
    sample as i16
}
