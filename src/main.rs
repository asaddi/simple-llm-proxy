#![warn(clippy::pedantic)]

use std::sync::Arc;

use anyhow::Result;
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::Response,
    response::IntoResponse,
    routing::{get, post},
};
use clap::Parser;
use reqwest::{Client, StatusCode, header};
use serde_json::{Value, json};
use tracing::{Level, event};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

const DEFAULT_BASE_URL: &str = "http://localhost:8080/v1";

#[derive(Debug, Clone, Parser)]
#[command(name = "simple-llm-proxy")]
struct Args {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value = "3000")]
    port: u16,
    #[arg(long)]
    base_url: Option<String>,
    #[arg(long)]
    base_url_env: Option<String>,
    #[arg(long)]
    api_key: Option<String>,
    #[arg(long)]
    api_key_env: Option<String>,
}

#[derive(Debug, Clone)]
struct ModelGateway {
    client: Client,
    base_url: String,
}

impl ModelGateway {
    pub fn new(base_url: &str, api_key: Option<&str>) -> Self {
        let base_url = base_url.trim_end_matches('/');
        let mut headers = header::HeaderMap::new();
        if let Some(key) = api_key {
            let mut value =
                header::HeaderValue::from_str(format!("Bearer {key}").as_str()).unwrap();
            value.set_sensitive(true);
            headers.append(header::AUTHORIZATION, value);
        }
        Self {
            client: Client::builder().default_headers(headers).build().unwrap(),
            base_url: base_url.to_owned(),
        }
    }

    fn endpoint(&self, path: &str) -> String {
        // TODO there's probably a way to ensure the result is still valid
        let mut result = String::new();
        result.push_str(&self.base_url);
        result.push_str(path);
        result
    }

    async fn streaming_aware_proxy(
        &self,
        path: &str,
        Json(payload): Json<Value>,
    ) -> Result<Response<Body>> {
        let resp = self
            .client
            .post(self.endpoint(path))
            .json(&payload)
            .send()
            .await?;
        Ok(Response::new(Body::from_stream(resp.bytes_stream())))
    }

    async fn model_handler(&self) -> Result<Response<Body>> {
        let resp = self.client.get(self.endpoint("/models")).send().await?;
        Ok(Response::new(Body::from_stream(resp.bytes_stream())))
    }
}

async fn chat_handler(
    State(state): State<Arc<ModelGateway>>,
    Json(payload): Json<Value>,
) -> (StatusCode, impl IntoResponse) {
    match state
        .streaming_aware_proxy("/chat/completions", Json(payload))
        .await
    {
        Ok(resp) => (resp.status(), resp),
        Err(e) => {
            event!(Level::ERROR, "streaming_aware_proxy failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":{"message":e.to_string()}})).into_response(),
            )
        }
    }
}

async fn model_handler(State(state): State<Arc<ModelGateway>>) -> (StatusCode, impl IntoResponse) {
    match state.model_handler().await {
        Ok(resp) => (resp.status(), resp),
        Err(e) => {
            event!(Level::ERROR, "model_handler failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error":{"message":e.to_string()}})).into_response(),
            )
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let fmt_layer = fmt::layer().with_target(false);
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .init();

    let args = Args::parse();
    let bind_addr = format!("{}:{}", args.host, args.port);

    let base_url = match args.base_url {
        Some(val) => val,
        None => match args.base_url_env {
            Some(var) => std::env::var(var)?,
            None => DEFAULT_BASE_URL.to_owned(),
        },
    };

    let api_key = match args.api_key {
        Some(val) => Some(val),
        None => match args.api_key_env {
            Some(var) => Some(std::env::var(var)?),
            None => None,
        },
    };

    let model_gateway = ModelGateway::new(base_url.as_str(), api_key.as_deref());

    event!(
        Level::INFO,
        "Listening on {bind_addr}; forwarding to {}",
        &model_gateway.base_url
    );

    let shared_model_gateway = Arc::new(model_gateway);

    let app = Router::new()
        .route("/v1/models", get(model_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .with_state(shared_model_gateway);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    event!(Level::INFO, "Exiting...");

    Ok(())
}
