use bytes::Bytes;

use eventsource_stream::Eventsource;
use futures::StreamExt;
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio_stream::wrappers::UnboundedReceiverStream;
use uuid::Uuid;

use crate::response::{MockErrorResponse, MockResponse, MockResponseBlock, MockScriptEntry};

pub enum ProviderFormat {
    OpenAI,
    Anthropic,
}

#[derive(Default)]
struct ToolCallAccumulator {
    id: String,
    name: String,
    arguments: String,
}

pub async fn process_and_save(
    rx: UnboundedReceiver<Bytes>,
    request_payload: Option<serde_json::Value>,
    provider: ProviderFormat,
    status_code: u16,
    out_dir: PathBuf,
) -> anyhow::Result<()> {
    if !status_code.to_string().starts_with('2') {
        // Handle direct error case without full SSE parsing
        let mut all_bytes = Vec::new();
        let mut rx = rx;
        while let Some(chunk) = rx.recv().await {
            all_bytes.extend_from_slice(&chunk);
        }
        let msg = String::from_utf8_lossy(&all_bytes).to_string();
        let entry = MockScriptEntry::Error {
            description: None,
            error: MockErrorResponse {
                status: status_code,
                message: msg,
            },
            request_payload,
        };
        save_entry(entry, out_dir).await?;
        return Ok(());
    }

    let stream = UnboundedReceiverStream::new(rx).map(Ok::<_, Infallible>);
    let mut event_stream = stream.eventsource();

    let mut current_text = String::new();
    let mut current_thinking = String::new();
    let mut tool_calls: HashMap<usize, ToolCallAccumulator> = HashMap::new();
    let mut ordered_blocks: Vec<MockResponseBlock> = Vec::new();

    let flush_content = |text: &mut String, thinking: &mut String, blocks: &mut Vec<MockResponseBlock>| {
        if !text.is_empty() {
            blocks.push(MockResponseBlock::Text {
                text: text.clone(),
            });
            text.clear();
        }
        if !thinking.is_empty() {
            blocks.push(MockResponseBlock::Thinking {
                text: thinking.clone(),
            });
            thinking.clear();
        }
    };

    match provider {
        ProviderFormat::OpenAI => {
            while let Some(Ok(event)) = event_stream.next().await {
                if event.data == "[DONE]" {
                    break;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&event.data) {
                    if let Some(choices) = v["choices"].as_array() {
                        for choice in choices {
                            let delta = &choice["delta"];

                            // Text delta
                            if let Some(text) = delta["content"].as_str() {
                                current_text.push_str(text);
                            }

                            // Tool Calls
                            if let Some(calls) = delta["tool_calls"].as_array() {
                                flush_content(&mut current_text, &mut current_thinking, &mut ordered_blocks);

                                for call in calls {
                                    if let Some(index) = call["index"].as_u64() {
                                        let idx = index as usize;
                                        let accum = tool_calls.entry(idx).or_default();

                                        if let Some(id) = call["id"].as_str() {
                                            accum.id = id.to_string();
                                        }
                                        if let Some(name) =
                                            call.pointer("/function/name").and_then(|v| v.as_str())
                                        {
                                            accum.name = name.to_string();
                                        }
                                        if let Some(args) = call
                                            .pointer("/function/arguments")
                                            .and_then(|v| v.as_str())
                                        {
                                            accum.arguments.push_str(args);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        ProviderFormat::Anthropic => {
            while let Some(Ok(event)) = event_stream.next().await {
                let v: Option<serde_json::Value> = serde_json::from_str(&event.data).ok();
                if let Some(v) = v {
                    match event.event.as_str() {
                        "content_block_start" => {
                            flush_content(&mut current_text, &mut current_thinking, &mut ordered_blocks);
                            if v.pointer("/content_block/type").and_then(|v| v.as_str())
                                == Some("tool_use")
                            {
                                if let Some(index) = v["index"].as_u64() {
                                    let accum = tool_calls.entry(index as usize).or_default();
                                    if let Some(id) =
                                        v.pointer("/content_block/id").and_then(|v| v.as_str())
                                    {
                                        accum.id = id.to_string();
                                    }
                                    if let Some(name) =
                                        v.pointer("/content_block/name").and_then(|v| v.as_str())
                                    {
                                        accum.name = name.to_string();
                                    }
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = v.get("delta") {
                                if let Some(text) = delta["text"].as_str() {
                                    current_text.push_str(text);
                                } else if let Some(thinking) = delta["thinking"].as_str() {
                                    current_thinking.push_str(thinking);
                                } else if let Some(partial) = delta["partial_json"].as_str() {
                                    if let Some(index) = v["index"].as_u64() {
                                        if let Some(accum) = tool_calls.get_mut(&(index as usize)) {
                                            accum.arguments.push_str(partial);
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    flush_content(&mut current_text, &mut current_thinking, &mut ordered_blocks);

    // Sort tool calls by index to maintain order and convert to blocks
    let mut tool_call_indices: Vec<_> = tool_calls.keys().copied().collect();
    tool_call_indices.sort_unstable();

    for idx in tool_call_indices {
        let accum = tool_calls.remove(&idx).unwrap();
        let input: serde_json::Value =
            serde_json::from_str(&accum.arguments).unwrap_or_else(|_| serde_json::json!({}));
        ordered_blocks.push(MockResponseBlock::ToolCall {
            id: accum.id,
            name: accum.name,
            input,
        });
    }

    let response = MockResponse {
        blocks: ordered_blocks,
        usage: None,
    };

    let entry = MockScriptEntry::Response {
        description: None,
        response,
        request_payload,
    };

    save_entry(entry, out_dir).await?;
    Ok(())
}

async fn save_entry(entry: MockScriptEntry, out_dir: PathBuf) -> anyhow::Result<()> {
    if !out_dir.exists() {
        tokio::fs::create_dir_all(&out_dir).await?;
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let full_uuid = Uuid::new_v4().as_simple().to_string();
    let short_uuid = if full_uuid.len() >= 8 { &full_uuid[0..8] } else { &full_uuid };
    let file_name = format!("{}_{}.yaml", timestamp, short_uuid);
    let path = out_dir.join(file_name);

    // We store it as an array to be compatible with single-file multi-entry reading formats
    let yaml = serde_yaml::to_string(&vec![entry])?;
    tokio::fs::write(&path, yaml).await?;
    println!("Recorded fixture: {:?}", path);
    Ok(())
}
