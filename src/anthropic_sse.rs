use crate::response::{MockResponse, MockResponseBlock};
use axum::response::sse::Event;
use serde_json::json;

/// Generates Anthropic SSE events from a MockResponse.
pub fn generate_events(resp: MockResponse) -> Vec<Event> {
    let mut events = Vec::new();

    let usage = resp.usage.unwrap_or_default();

    // 1. message_start
    events.push(
        Event::default().event("message_start").data(
            json!({
                "type": "message_start",
                "message": {
                    "id": "msg_mock_123",
                    "type": "message",
                    "role": "assistant",
                    "content": [],
                    "model": "mock",
                    "usage": {
                        "input_tokens": usage.input_tokens,
                        "output_tokens": 0,
                        "cache_creation_input_tokens": usage.cache_creation_input_tokens,
                        "cache_read_input_tokens": usage.cache_read_input_tokens
                    }
                }
            })
            .to_string(),
        ),
    );

    let mut has_tool = false;

    // 2. content blocks
    for (index, block) in resp.blocks.into_iter().enumerate() {
        match block {
            MockResponseBlock::Text { text } => {
                events.push(
                    Event::default().event("content_block_start").data(
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": { "type": "text", "text": "" }
                        })
                        .to_string(),
                    ),
                );

                events.push(
                    Event::default().event("content_block_delta").data(
                        json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": { "type": "text_delta", "text": text }
                        })
                        .to_string(),
                    ),
                );

                events.push(
                    Event::default().event("content_block_stop").data(
                        json!({
                            "type": "content_block_stop",
                            "index": index
                        })
                        .to_string(),
                    ),
                );
            }
            MockResponseBlock::Thinking { text } => {
                events.push(
                    Event::default().event("content_block_start").data(
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": { "type": "thinking", "thinking": "" }
                        })
                        .to_string(),
                    ),
                );

                events.push(
                    Event::default().event("content_block_delta").data(
                        json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": { "type": "thinking_delta", "thinking": text }
                        })
                        .to_string(),
                    ),
                );

                events.push(
                    Event::default().event("content_block_stop").data(
                        json!({
                            "type": "content_block_stop",
                            "index": index
                        })
                        .to_string(),
                    ),
                );
            }
            MockResponseBlock::ToolCall { id, name, input } => {
                has_tool = true;
                events.push(
                    Event::default().event("content_block_start").data(
                        json!({
                            "type": "content_block_start",
                            "index": index,
                            "content_block": {
                                "type": "tool_use",
                                "id": id,
                                "name": name,
                                "input": {}
                            }
                        })
                        .to_string(),
                    ),
                );

                events.push(
                    Event::default().event("content_block_delta").data(
                        json!({
                            "type": "content_block_delta",
                            "index": index,
                            "delta": {
                                "type": "input_json_delta",
                                "partial_json": serde_json::to_string(&input).unwrap_or_default()
                            }
                        })
                        .to_string(),
                    ),
                );

                events.push(
                    Event::default().event("content_block_stop").data(
                        json!({
                            "type": "content_block_stop",
                            "index": index
                        })
                        .to_string(),
                    ),
                );
            }
        }
    }

    // 3. message_delta
    let stop_reason = if has_tool { "tool_use" } else { "end_turn" };
    events.push(
        Event::default().event("message_delta").data(
            json!({
                "type": "message_delta",
                "delta": { "stop_reason": stop_reason },
                "usage": { "output_tokens": usage.output_tokens }
            })
            .to_string(),
        ),
    );

    // 4. message_stop
    events.push(
        Event::default()
            .event("message_stop")
            .data(json!({ "type": "message_stop" }).to_string()),
    );

    events
}
