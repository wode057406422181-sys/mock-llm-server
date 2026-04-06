use crate::decoder::{self, ProviderFormat};
use axum::{
    body::Body,
    extract::{Request, State},
    http::StatusCode,
    response::IntoResponse,
};
use bytes::Bytes;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Clone)]
pub struct ProxyState {
    pub openai_url: String,
    pub anthropic_url: String,
    pub out_dir: PathBuf,
    pub client: reqwest::Client,
}

pub async fn handle_proxy_openai(
    State(state): State<Arc<ProxyState>>,
    request: Request<Body>,
) -> axum::response::Result<impl IntoResponse, StatusCode> {
    forward_and_record(state, request, ProviderFormat::OpenAI).await
}

pub async fn handle_proxy_anthropic(
    State(state): State<Arc<ProxyState>>,
    request: Request<Body>,
) -> axum::response::Result<impl IntoResponse, StatusCode> {
    forward_and_record(state, request, ProviderFormat::Anthropic).await
}

async fn forward_and_record(
    state: Arc<ProxyState>,
    request: Request<Body>,
    provider: ProviderFormat,
) -> axum::response::Result<impl IntoResponse, StatusCode> {
    let method = request.method().clone();
    let (parts, body) = request.into_parts();

    // Read the entire request body to save it (max 10 MB limit)
    const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;
    let req_bytes = axum::body::to_bytes(body, MAX_BODY_SIZE)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let req_json: Option<serde_json::Value> = serde_json::from_slice(&req_bytes).ok();

    // Construct upstream URL
    let target_base = match provider {
        ProviderFormat::OpenAI => &state.openai_url,
        ProviderFormat::Anthropic => &state.anthropic_url,
    };
    let path_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("");
    let url = format!("{}{}", target_base.trim_end_matches('/'), path_query);

    // Forward headers
    let mut headers = HeaderMap::new();
    for (name, value) in parts.headers.iter() {
        // Skip host and hop-by-hop
        if name != reqwest::header::HOST && name != reqwest::header::CONNECTION {
            if let Ok(name) = HeaderName::from_bytes(name.as_str().as_bytes()) {
                if let Ok(val) = HeaderValue::from_bytes(value.as_bytes()) {
                    headers.insert(name, val);
                }
            }
        }
    }

    // Attempt to make the request
    let resp = state
        .client
        .request(method, &url)
        .headers(headers)
        .body(req_bytes)
        .send()
        .await
        .map_err(|e| {
            eprintln!("Proxy upstream request failed: {}", e);
            StatusCode::BAD_GATEWAY
        })?;

    let status = resp.status();
    let mut resp_headers = axum::http::HeaderMap::new();
    for (name, value) in resp.headers().iter() {
        if let Ok(n) = axum::http::HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(v) = axum::http::HeaderValue::from_bytes(value.as_bytes()) {
                resp_headers.insert(n, v);
            }
        }
    }

    let (tx, rx) = mpsc::unbounded_channel::<Bytes>();

    // Background task to consume captured SSE and write fixture
    tokio::spawn(async move {
        if let Err(e) = decoder::process_and_save(
            rx,
            req_json,
            provider,
            status.as_u16(),
            state.out_dir.clone(),
        )
        .await
        {
            eprintln!("Proxy decoder error: {}", e);
        }
    });

    let stream = resp.bytes_stream().map(move |res| match res {
        Ok(b) => {
            let _ = tx.send(b.clone());
            Ok::<_, Infallible>(b)
        }
        Err(_) => Ok::<_, Infallible>(Bytes::new()),
    });

    let mut response_builder = axum::response::Response::builder().status(status.as_u16());
    *response_builder.headers_mut().unwrap() = resp_headers;

    let axum_body = Body::from_stream(stream);
    Ok(response_builder.body(axum_body).unwrap())
}
