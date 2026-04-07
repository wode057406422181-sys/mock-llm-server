use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, sse::Sse},
    routing::post,
};
use futures::StreamExt;
use futures::stream;
use serde_json::json;
use std::collections::VecDeque;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::anthropic_sse;
use crate::openai_sse;
use crate::response::{MockErrorResponse, MockScriptEntry};

use axum::routing::get;

pub struct AppState {
    pub script: Mutex<VecDeque<MockScriptEntry>>,
    pub original_script: Vec<MockScriptEntry>,
    pub loop_mode: bool,
}

pub struct MockLlmServer {
    addr: SocketAddr,
    state: Arc<AppState>,
    handle: JoinHandle<()>,
}

impl MockLlmServer {
    /// Start Mock Server on a random available port
    pub async fn start(script: Vec<MockScriptEntry>) -> anyhow::Result<Self> {
        let state = Arc::new(AppState {
            script: Mutex::new(VecDeque::from(script.clone())),
            original_script: script,
            loop_mode: false,
        });

        let app = Self::create_router(state.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        // Spawn server
        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                eprintln!("Mock server error: {}", e);
            }
        });

        Ok(Self {
            addr,
            state,
            handle,
        })
    }

    pub fn create_router(state: Arc<AppState>) -> Router {
        Router::new()
            .route("/v1/models", get(handle_models))
            .route("/v1/messages", post(handle_anthropic))
            .route("/v1/chat/completions", post(handle_openai))
            .with_state(state)
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }

    pub async fn remaining(&self) -> usize {
        let q = self.state.script.lock().await;
        q.len()
    }

    pub async fn shutdown(self) {
        self.handle.abort();
        let _ = self.handle.await;
    }
}

async fn get_next_entry(
    state: State<Arc<AppState>>,
) -> Result<MockScriptEntry, (StatusCode, Json<serde_json::Value>)> {
    let mut q = state.script.lock().await;

    if q.is_empty() && state.loop_mode {
        *q = VecDeque::from(state.original_script.clone());
    }

    if let Some(entry) = q.pop_front() {
        Ok(entry)
    } else {
        Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "No more mock responses in script" })),
        ))
    }
}

async fn handle_models() -> Json<serde_json::Value> {
    Json(json!({
        "object": "list",
        "data": [
            {
                "id": "mock-model",
                "object": "model",
                "created": 1686935002,
                "owned_by": "mock"
            }
        ]
    }))
}

async fn handle_anthropic(
    state: State<Arc<AppState>>,
) -> axum::response::Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let entry = get_next_entry(state).await?;

    match entry {
        MockScriptEntry::Response { response: resp, .. } => {
            let events = anthropic_sse::generate_events(resp);
            let stream = stream::iter(events).then(|event| async move {
                // ADDED: simulate real typing delay! 
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                Ok::<_, Infallible>(event)
            });
            Ok(Sse::new(stream).into_response())
        }
        MockScriptEntry::Error {
            error: MockErrorResponse { status, message },
            ..
        } => {
            let status_code =
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            Err((
                status_code,
                Json(json!({
                    "error": { "type": "api_error", "message": message }
                })),
            ))
        }
    }
}

async fn handle_openai(
    state: State<Arc<AppState>>,
) -> axum::response::Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    let entry = get_next_entry(state).await?;

    match entry {
        MockScriptEntry::Response { response: resp, .. } => {
            let events = openai_sse::generate_events(resp);
            let stream = stream::iter(events).then(|event| async move {
                // ADDED: simulate real typing delay! 
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                Ok::<_, Infallible>(event)
            });
            Ok(Sse::new(stream).into_response())
        }
        MockScriptEntry::Error {
            error: MockErrorResponse { status, message },
            ..
        } => {
            let status_code =
                StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            Err((
                status_code,
                Json(json!({
                    "error": { "message": message }
                })),
            ))
        }
    }
}
