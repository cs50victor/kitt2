use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
        CreateChatCompletionRequestArgs,
    },
    Client,
};
use bevy::ecs::{
    system::{Res, ResMut, Resource},
    world::{FromWorld, World},
};
use futures::StreamExt;

use crate::OPENAI_ORG_ID_ENV;

#[derive(Resource)]
pub struct LLMChannel {
    pub tx: crossbeam_channel::Sender<String>,
    pub rx: crossbeam_channel::Receiver<String>,
    pub client: Client<OpenAIConfig>,
}

impl FromWorld for LLMChannel {
    fn from_world(_world: &mut World) -> Self {
        let open_ai_org_id = std::env::var(OPENAI_ORG_ID_ENV).unwrap();

        let (tx, rx) = crossbeam_channel::unbounded::<String>();

        let openai_client = async_openai::Client::with_config(
            async_openai::config::OpenAIConfig::new().with_org_id(open_ai_org_id),
        );

        Self { tx, rx, client: openai_client }
    }
}

/// Stream text chunks to gpt as it's being generated.
/// Note: if chunks don't end with space or punctuation (" ", ".", "?", "!"),
/// the stream will wait for more text.
/// Used during input streaming to chunk text blocks and set last char to space
pub fn run_llm(
    llm_channel: ResMut<LLMChannel>,
    async_runtime: Res<crate::AsyncRuntime>,
    // mut tts_client: Res<crate::tts::TTS>,
) {
    let splitters = ['.', ',', '?', '!', ';', ':', '—', '-', ')', ']', '}', ' '];

    let mut txt_buffer = String::new();
    let mut tts_buffer = String::new();

    let mut req_args = CreateChatCompletionRequestArgs::default();
    let openai_req = req_args.model("gpt-4-1106-preview").max_tokens(512u16);

    let text_chat_prefix = "[chat]";
    // let text_latency = Duration::from_millis(500);

    while let Ok(chunk) = llm_channel.rx.try_recv() {
        log::info!("\n\n\nchunk gotten from llm channel");
        txt_buffer.push_str(&chunk);
        if txt_buffer.starts_with(text_chat_prefix) || ends_with_splitter(&splitters, &txt_buffer) {
            let request = openai_req
                .messages([ChatCompletionRequestUserMessageArgs::default()
                    .content(ChatCompletionRequestUserMessageContent::Text(remove_prefix(
                        &txt_buffer,
                        text_chat_prefix,
                    )))
                    .build()
                    .unwrap()
                    .into()])
                .build()
                .unwrap();

            async_runtime.rt.block_on(async {
                let mut gpt_resp_stream =
                    llm_channel.client.chat().create_stream(request).await.unwrap();
                while let Some(result) = gpt_resp_stream.next().await {
                    match result {
                        Ok(response) => {
                            for chat_choice in response.choices {
                                if let Some(content) = chat_choice.delta.content {
                                    tts_buffer.push_str(&content);
                                    if ends_with_splitter(&splitters, &tts_buffer) {
                                        let msg = {
                                            let txt = tts_buffer.clone();
                                            txt.trim().to_owned()
                                        };
                                        log::info!("GPT: {msg}");
                                        // if let Err(e) = tts_client.send(msg) {
                                        //     error!("Coudln't send gpt text chunk to tts channel - {e}");
                                        // } else {
                                        //     tts_buffer.clear();
                                        // };
                                    }
                                };
                            }
                        },
                        Err(err) => {
                            log::warn!("chunk error: {err:#?}");
                        },
                    }
                }
            });
            txt_buffer.clear();
        }
    }
}

// ***** HELPER FUNCTIONS *****

fn ends_with_splitter(splitters: &[char], chunk: &str) -> bool {
    !chunk.is_empty() && chunk != " " && splitters.iter().any(|&splitter| chunk.ends_with(splitter))
}

fn remove_prefix(s: &str, prefix: &str) -> String {
    let s = match s.strip_prefix(prefix) {
        Some(s) => s,
        None => s,
    };
    s.to_owned()
}
