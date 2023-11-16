use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use futures::StreamExt;
use livekit::webrtc::audio_stream::native::NativeAudioStream;
use log::error;
use tokio::sync::mpsc::UnboundedSender;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperError};

pub struct STT {
    whisper_ctx: WhisperContext,
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
    pub const STEP_MS: i32 = 3000;

    pub fn new() -> anyhow::Result<Self> {
        // ggml-base.en.bin
        let whisper_ctx = WhisperContext::new("./ggml-tiny.bin")?;
        Ok(Self { whisper_ctx })
    }

    pub fn gen_whisper_params<'b>(&self) -> FullParams<'b, 'b> {
        let mut params = FullParams::new(SamplingStrategy::default());
        params.set_print_progress(false);
        params.set_print_special(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        params.set_suppress_blank(true);
        params.set_language(Some("en"));
        params.set_token_timestamps(false);
        params.set_duration_ms(Self::LATENCY_MS as i32);
        params.set_no_context(true);

        params.set_speed_up(false);
        params.set_translate(false);
        params.set_audio_ctx(0);
        params.set_max_tokens(32);
        params.set_n_threads(2);
        //params.set_no_speech_thold(0.3);
        //params.set_split_on_word(true);

        params
    }
}

pub async fn transcribe(
    txt_input_sender: UnboundedSender<String>,
    tts_client: Arc<STT>,
    mut audio_stream: NativeAudioStream,
) -> Result<(), WhisperError> {
    let mut audio_buffer: Vec<i16> = Vec::new();
    let mut state = tts_client.whisper_ctx.create_state()?;

    let mut start_time = Instant::now();

    while let Some(frame) = audio_stream.next().await {
        audio_buffer.extend_from_slice(&frame.data);
        let audio_samples = whisper_rs::convert_integer_to_float_audio(&audio_buffer);
        if audio_buffer.len() >= (STT::LATENCY_SAMPLES as usize)
            || start_time.elapsed() >= Duration::from_millis(STT::LATENCY_MS as u64)
        {
            start_time = Instant::now();

            state.full(tts_client.gen_whisper_params(), &audio_samples)?;
            let num_tokens = state.full_n_tokens(0)?;
            let transcription = (1..num_tokens - 1)
                .map(|i| {
                    state.full_get_token_text(0, i).unwrap_or_else(|err| {
                        // Handle the error, e.g., log it, and continue with a default value or exit
                        // For example, using a default value like an empty string
                        error!("couldn't processing token {}: {}", i, err);
                        String::new()
                    })
                })
                .collect::<String>();

            if let Err(e) = txt_input_sender.send(transcription) {
                error!("couldn't send transcription to gpt {e:?}")
            };

            audio_buffer.clear();
        }
    }
    Ok(())
}
