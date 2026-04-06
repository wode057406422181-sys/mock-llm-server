use axum::serve;
use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use clap::{Parser, Subcommand};
use mock_llm_server::{
    mock_server::{AppState, MockLlmServer},
    response::MockResponse,
};

#[derive(Parser, Debug)]
#[command(author, version, about = "Mock LLM Server and Recorder Utility")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the mock LLM API server
    Serve {
        /// Port to listen on. Default: 3000
        #[arg(short, long, default_value_t = 3000)]
        port: u16,

        /// Script file or directory to load from. If missing, runs with empty script.
        #[arg(short, long)]
        script: Option<PathBuf>,

        /// Enable loop mode (restart script sequence from beginning when exhausted)
        #[arg(short, long)]
        loop_mode: bool,
    },
    /// Start as a transparent reverse proxy that records API traffic into YAML fixtures
    Proxy {
        /// Port to listen on. Default: 8080
        #[arg(short, long, default_value_t = 8080)]
        port: u16,

        /// Real Anthropic API Base URL for forwarding
        #[arg(long, default_value = "https://api.anthropic.com")]
        anthropic_url: String,

        /// Real OpenAI API Base URL for forwarding
        #[arg(long, default_value = "https://api.openai.com")]
        openai_url: String,

        /// Output directory for saving recorded fixtures
        #[arg(short, long, default_value = "fixtures_tmp")]
        out_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging with `info` level default.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "mock_llm_server=debug,tower_http=debug".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve {
            port,
            script,
            loop_mode,
        } => {
            let entries = if let Some(path) = &script {
                if path.is_dir() {
                    println!("Loading scripts from directory: {:?}", path);
                    MockResponse::load_script_from_dir(path).await?
                } else {
                    println!("Loading script from yaml file: {:?}", path);
                    MockResponse::load_script_from_yaml(path).await?
                }
            } else {
                println!(
                    "Warning: No script provided. Mock server will just return 500 automatically."
                );
                vec![]
            };

            println!("Loaded {} MockScriptEntries.", entries.len());

            let state = Arc::new(AppState {
                script: Mutex::new(VecDeque::from(entries.clone())),
                original_script: entries,
                loop_mode,
            });

            let router = MockLlmServer::create_router(state);

            let app = {
                use tower_http::{cors::CorsLayer, trace::TraceLayer};
                router
                    .layer(CorsLayer::permissive())
                    .layer(TraceLayer::new_for_http())
            };

            let addr = SocketAddr::from(([0, 0, 0, 0], port));
            println!("Mock LLM Server listening on http://{}", addr);
            if loop_mode {
                println!(
                    "Loop mode enabled. (When scripts deplete, they will reset to beginning.)"
                );
            }

            let listener = tokio::net::TcpListener::bind(addr).await?;
            serve(listener, app).await?;
        }

        Commands::Proxy {
            port,
            anthropic_url,
            openai_url,
            out_dir,
        } => {
            use axum::{
                Json, Router,
                routing::{get, post},
            };
            println!("Starting Proxy Recorder...");

            let state = Arc::new(mock_llm_server::proxy::ProxyState {
                openai_url,
                anthropic_url,
                out_dir: out_dir.clone(),
                client: reqwest::Client::new(),
            });

            if !out_dir.exists() {
                tokio::fs::create_dir_all(&out_dir).await?;
            }

            let app = Router::new()
                .route("/v1/models", get(|| async {
                    Json(serde_json::json!({
                        "object": "list",
                        "data": [{ "id": "mock-model", "object": "model", "created": 1686935002, "owned_by": "mock" }]
                    }))
                }))
                .route("/v1/messages", post(mock_llm_server::proxy::handle_proxy_anthropic))
                .route("/v1/chat/completions", post(mock_llm_server::proxy::handle_proxy_openai))
                .with_state(state);

            let app = {
                use tower_http::{cors::CorsLayer, trace::TraceLayer};
                app.layer(CorsLayer::permissive())
                    .layer(TraceLayer::new_for_http())
            };

            let addr = SocketAddr::from(([0, 0, 0, 0], port));
            println!(
                "Proxy listening on http://{} -> intercepting & recording to {:?}",
                addr, out_dir
            );

            let listener = tokio::net::TcpListener::bind(addr).await?;
            serve(listener, app).await?;
        }
    }

    Ok(())
}
