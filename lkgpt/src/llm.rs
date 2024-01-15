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

use crate::OPENAI_ORG_ID;

#[derive(Resource)]
pub struct LLMChannel {
    pub tx: crossbeam_channel::Sender<String>,
    pub rx: crossbeam_channel::Receiver<String>,
    pub client: Client<OpenAIConfig>,
    pub splitters: [char; 12],
    pub txt_buffer: String,
    pub tts_buffer: String,
    pub req_args: CreateChatCompletionRequestArgs,
    pub text_chat_prefix: &'static str,
}

impl Default for LLMChannel {
    fn default() -> Self {
        let open_ai_org_id = std::env::var(OPENAI_ORG_ID).unwrap();

        let (tx, rx) = crossbeam_channel::unbounded::<String>();

        let openai_client = async_openai::Client::with_config(
            async_openai::config::OpenAIConfig::new().with_org_id(open_ai_org_id),
        );

        let splitters = ['.', ',', '?', '!', ';', ':', '—', '-', ')', ']', '}', ' '];

        let txt_buffer = String::new();
        let tts_buffer = String::new();

        let req_args = CreateChatCompletionRequestArgs::default();

        let text_chat_prefix = "[chat]";

        Self {
            tx,
            rx,
            client: openai_client,
            splitters,
            txt_buffer,
            tts_buffer,
            req_args,
            text_chat_prefix,
        }
    }
}

impl LLMChannel {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Stream text chunks to gpt as it's being generated.
/// Note: if chunks don't end with space or punctuation (" ", ".", "?", "!"),
/// the stream will wait for more text.
/// Used during input streaming to chunk text blocks and set last char to space
pub fn run_llm(
    mut llm_channel: ResMut<LLMChannel>,
    async_runtime: Res<crate::AsyncRuntime>,
    mut tts_client: ResMut<crate::tts::TTS>,
) {
    while let Ok(chunk) = llm_channel.rx.try_recv() {
        log::info!("\n\n\nchunk gotten from llm channel, {chunk}");
        llm_channel.txt_buffer.push_str(&chunk);
        if llm_channel.txt_buffer.starts_with(llm_channel.text_chat_prefix)
            || ends_with_splitter(&llm_channel.splitters, &llm_channel.txt_buffer)
        {
            let (txt_buffer, prefix) =
                (llm_channel.txt_buffer.clone(), llm_channel.text_chat_prefix);
            let request = llm_channel
                .req_args
                .model("gpt-4-1106-preview")
                .max_tokens(512u16)
                .messages([ChatCompletionRequestUserMessageArgs::default()
                    .content(ChatCompletionRequestUserMessageContent::Text(remove_prefix(
                        txt_buffer.as_str(),
                        prefix,
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
                                    llm_channel.tts_buffer.push_str(&content);
                                    if ends_with_splitter(
                                        &llm_channel.splitters,
                                        &llm_channel.tts_buffer,
                                    ) {
                                        let msg = {
                                            let txt = llm_channel.tts_buffer.clone();
                                            txt.trim().to_owned()
                                        };
                                        log::info!("GPT: {msg}");
                                        if let Err(e) = tts_client.send(msg) {
                                            log::error!(
                                                "Coudln't send gpt text chunk to tts channel - {e}"
                                            );
                                        } else {
                                            llm_channel.tts_buffer.clear();
                                        };
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
            llm_channel.txt_buffer.clear();
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
