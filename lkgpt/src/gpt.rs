use std::time::{Duration, Instant};

use async_openai::{
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestUserMessageArgs, ChatCompletionRequestUserMessageContent,
        CreateChatCompletionRequestArgs,
    },
    Client,
};
use futures::StreamExt;
use log::{error, info, warn};
use tokio::{sync::mpsc, time};

use crate::{stt::STT, tts::TTS};

/// Stream text chunks to gpt as it's being generated, with <1s latency.
/// Note: if chunks don't end with space or punctuation (" ", ".", "?", "!"),
/// the stream will wait for more text.
/// Used during input streaming to chunk text blocks and set last char to space
pub async fn gpt(
    mut text_input_rx: mpsc::UnboundedReceiver<String>,
    openai_client: Client<OpenAIConfig>,
    mut tts_client: TTS,
) -> anyhow::Result<()> {
    let splitters = [
        '.', ',', '?', '!', ';', ':', '—', '-', '(', ')', '[', ']', '}', ' ',
    ];

    let mut txt_buffer = String::new();
    let mut tts_buffer = String::new();

    let mut req_args = CreateChatCompletionRequestArgs::default();
    let openai_req = req_args.model("gpt-3.5-turbo").max_tokens(512u16);

    // let text_latency = Duration::from_millis(500);
    while let Some(chunk) = text_input_rx.recv().await {
        txt_buffer.push_str(&chunk);
        if ends_with_splitter(&splitters, &txt_buffer){
            let request = openai_req
                .messages([ChatCompletionRequestUserMessageArgs::default()
                    .content(ChatCompletionRequestUserMessageContent::Text(
                        txt_buffer.clone(),
                    ))
                    .build()?
                    .into()])
                .build()?;

            let mut gpt_resp_stream = openai_client.chat().create_stream(request).await?;
            while let Some(result) = gpt_resp_stream.next().await {
                match result {
                    Ok(response) => {
                        for chat_choice in response.choices {
                            if let Some(content) = chat_choice.delta.content {
                                tts_buffer.push_str(&content);
                                if ends_with_splitter(&splitters, &tts_buffer) {
                                    if let Err(e) = tts_client.send(tts_buffer.clone()) {
                                        error!("Coudln't send gpt text chunk to tts channel - {e}");
                                    } else {
                                        tts_buffer.clear();
                                    };
                                }
                            };
                        }
                    }
                    Err(err) => {
                        warn!("chunk error: {err:#?}");
                    }
                }
            }
            txt_buffer.clear();
        } else if !txt_buffer.ends_with(' ') {
            txt_buffer.push(' ');
        }
    }
    Ok(())
}

fn ends_with_splitter(splitters: &[char], chunk: &str) -> bool {
    !chunk.is_empty() && chunk != " " && splitters.iter().any(|&splitter| chunk.ends_with(splitter))
}
