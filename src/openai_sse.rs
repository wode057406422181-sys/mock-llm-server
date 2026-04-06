use crate::response::{MockResponse, MockResponseBlock};
use axum::response::sse::Event;
use serde_json::json;

/// Generates OpenAI SSE events from a MockResponse.
pub fn generate_events(resp: MockResponse) -> Vec<Event> {
    let mut events = Vec::new();

    let usage = resp.usage.unwrap_or_default();

    // 1. Initial chunk with role
    events.push(
        Event::default().data(
            json!({
                "id": "chatcmpl-mock",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant" },
                    "finish_reason": null
                }]
            })
            .to_string(),
        ),
    );

    let mut has_tool = false;
    let mut tool_index = 0;

    // 2. content blocks
    for block in resp.blocks {
        match block {
            MockResponseBlock::Text { text } | MockResponseBlock::Thinking { text } => {
                // OpenAI has no native "thinking" block in SSE format, treat it as text
                events.push(
                    Event::default().data(
                        json!({
                            "id": "chatcmpl-mock",
                            "object": "chat.completion.chunk",
                            "choices": [{
                                "index": 0,
                                "delta": { "content": text },
                                "finish_reason": null
                            }]
                        })
                        .to_string(),
                    ),
                );
            }
            MockResponseBlock::ToolCall { id, name, input } => {
                has_tool = true;
                events.push(
                    Event::default().data(json!({
                        "id": "chatcmpl-mock",
                        "object": "chat.completion.chunk",
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": tool_index,
                                    "id": id,
                                    "type": "function",
                                    "function": {
                                        "name": name,
                                        "arguments": serde_json::to_string(&input).unwrap_or_default()
                                    }
                                }]
                            },
                            "finish_reason": null
                        }]
                    }).to_string())
                );
                tool_index += 1;
            }
        }
    }

    // 3. Finish reason chunk
    let finish_reason = if has_tool { "tool_calls" } else { "stop" };
    events.push(
        Event::default().data(
            json!({
                "id": "chatcmpl-mock",
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {},
                    "finish_reason": finish_reason
                }]
            })
            .to_string(),
        ),
    );

    // 4. Usage chunk
    events.push(
        Event::default().data(
            json!({
                "id": "chatcmpl-mock",
                "object": "chat.completion.chunk",
                "choices": [],
                "usage": {
                    "prompt_tokens": usage.input_tokens,
                    "completion_tokens": usage.output_tokens,
                    "total_tokens": usage.input_tokens + usage.output_tokens
                }
            })
            .to_string(),
        ),
    );

    // 5. [DONE]
    events.push(Event::default().data("[DONE]"));

    events
}
