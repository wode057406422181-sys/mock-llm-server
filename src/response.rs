use serde::{Deserialize, Serialize};

/// A complete HTTP request-response script.
/// Contains one or more blocks, supporting mixed responses (text + tool, multi-tool parallel, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockResponse {
    /// All content blocks returned by an API call.
    /// The order matches the sequence of SSE events sent.
    pub blocks: Vec<MockResponseBlock>,
    /// Simulated usage information
    #[serde(default)]
    pub usage: Option<MockUsage>,
}

/// Single content block
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MockResponseBlock {
    /// Text content
    Text { text: String },
    /// Thinking content (Anthropic extended thinking)
    Thinking { text: String },
    /// Tool call
    ToolCall {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

/// Simulated API error (Direct HTTP error response, no SSE)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockErrorResponse {
    pub status: u16,
    pub message: String,
}

/// Simulated usage data
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MockUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
    #[serde(default)]
    pub cache_read_input_tokens: u64,
}

/// Entry in the script queue: normal response or error
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum MockScriptEntry {
    Response {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        description: Option<String>,
        #[serde(flatten)]
        response: MockResponse,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        request_payload: Option<serde_json::Value>,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        description: Option<String>,
        #[serde(flatten)]
        error: MockErrorResponse,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        request_payload: Option<serde_json::Value>,
    },
}

impl MockResponse {
    /// Helper: Pure text response
    pub fn text(s: impl Into<String>) -> MockScriptEntry {
        MockScriptEntry::Response {
            description: None,
            response: MockResponse {
                blocks: vec![MockResponseBlock::Text { text: s.into() }],
                usage: None,
            },
            request_payload: None,
        }
    }

    /// Helper: Pure text response with usage
    pub fn text_with_usage(
        s: impl Into<String>,
        input_tokens: u64,
        output_tokens: u64,
    ) -> MockScriptEntry {
        MockScriptEntry::Response {
            description: None,
            response: MockResponse {
                blocks: vec![MockResponseBlock::Text { text: s.into() }],
                usage: Some(MockUsage {
                    input_tokens,
                    output_tokens,
                    ..Default::default()
                }),
            },
            request_payload: None,
        }
    }

    /// Helper: Single tool call
    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> MockScriptEntry {
        MockScriptEntry::Response {
            description: None,
            response: MockResponse {
                blocks: vec![MockResponseBlock::ToolCall {
                    id: id.into(),
                    name: name.into(),
                    input,
                }],
                usage: None,
            },
            request_payload: None,
        }
    }

    /// Helper: Text + tool mixed
    pub fn text_then_tool(
        text: impl Into<String>,
        id: impl Into<String>,
        name: impl Into<String>,
        input: serde_json::Value,
    ) -> MockScriptEntry {
        MockScriptEntry::Response {
            description: None,
            response: MockResponse {
                blocks: vec![
                    MockResponseBlock::Text { text: text.into() },
                    MockResponseBlock::ToolCall {
                        id: id.into(),
                        name: name.into(),
                        input,
                    },
                ],
                usage: None,
            },
            request_payload: None,
        }
    }

    /// Helper: Multi-tool parallel
    pub fn multi_tool(tools: Vec<(String, String, serde_json::Value)>) -> MockScriptEntry {
        let blocks = tools
            .into_iter()
            .map(|(id, name, input)| MockResponseBlock::ToolCall { id, name, input })
            .collect();
        MockScriptEntry::Response {
            description: None,
            response: MockResponse {
                blocks,
                usage: None,
            },
            request_payload: None,
        }
    }

    /// Helper: API error
    pub fn error(status: u16, message: impl Into<String>) -> MockScriptEntry {
        MockScriptEntry::Error {
            description: None,
            error: MockErrorResponse {
                status,
                message: message.into(),
            },
            request_payload: None,
        }
    }

    /// Load Mock script from a YAML file
    pub async fn load_script_from_yaml(
        path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<Vec<MockScriptEntry>> {
        let content = tokio::fs::read_to_string(path).await?;
        let entries = serde_yaml::from_str(&content)?;
        Ok(entries)
    }

    /// Load all .yaml / .yml files from a directory in alphabetical order,
    /// combining them into a complete Mock script queue.
    pub async fn load_script_from_dir(
        dir_path: impl AsRef<std::path::Path>,
    ) -> anyhow::Result<Vec<MockScriptEntry>> {
        let mut entries = Vec::new();
        let mut paths = Vec::new();
        let mut dir = tokio::fs::read_dir(dir_path).await?;
        while let Ok(Some(entry)) = dir.next_entry().await {
            let p = entry.path();
            if p.is_file() {
                if p.extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| matches!(ext.to_lowercase().as_str(), "yaml" | "yml"))
                    .unwrap_or(false)
                {
                    paths.push(p);
                }
            }
        }

        paths.sort();

        for path in paths {
            let content = tokio::fs::read_to_string(&path).await?;
            // Try parsing as array
            if let Ok(vec) = serde_yaml::from_str::<Vec<MockScriptEntry>>(&content) {
                entries.extend(vec);
            } else {
                // Fallback to parsing as single entry if array parsing fails
                let single: MockScriptEntry = serde_yaml::from_str(&content)
                    .map_err(|e| anyhow::anyhow!("Failed to parse file {:?}: {}", path, e))?;
                entries.push(single);
            }
        }

        Ok(entries)
    }
}
