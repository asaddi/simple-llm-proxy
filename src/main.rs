#![warn(clippy::pedantic)]

use std::{
    collections::HashSet,
    sync::{Arc, LazyLock},
    time::Duration,
};

use anyhow::Result;
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderName, Response},
    response::IntoResponse,
    routing::{get, post},
};
use bon::bon;
use clap::Parser;
use reqwest::{Client, StatusCode, header};
use serde_json::{Value, json};
use tracing::{Level, event};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

const DEFAULT_BASE_URL: &str = "http://localhost:8080/v1";
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_mins(5);
const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_mins(10);

static HOP_BY_HOP_HEADERS: LazyLock<HashSet<HeaderName>> = LazyLock::new(|| {
    let mut headers = HashSet::new();
    headers.insert(header::TRANSFER_ENCODING);
    headers.insert(header::TE);
    headers.insert(header::CONNECTION);
    headers.insert(header::TRAILER);
    headers.insert(header::UPGRADE);
    headers.insert(header::PROXY_AUTHORIZATION);
    headers.insert(header::PROXY_AUTHENTICATE);
    headers.insert(HeaderName::from_static("keep-alive"));
    headers
});

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
struct LlmProxy {
    client: Client,
    base_url: String,
}

#[bon]
impl LlmProxy {
    #[builder]
    fn new(
        base_url: &str,
        api_key: Option<&str>,
        connect_timeout: Option<Duration>,
        read_timeout: Option<Duration>,
        total_timeout: Option<Duration>,
    ) -> Self {
        let base_url = base_url.trim_end_matches('/');
        let mut headers = header::HeaderMap::new();
        if let Some(key) = api_key {
            let mut value =
                header::HeaderValue::from_str(format!("Bearer {key}").as_str()).unwrap();
            value.set_sensitive(true);
            headers.append(header::AUTHORIZATION, value);
        }
        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT))
            .read_timeout(read_timeout.unwrap_or(DEFAULT_READ_TIMEOUT))
            .timeout(total_timeout.unwrap_or(DEFAULT_TOTAL_TIMEOUT))
            .build()
            .unwrap();
        Self {
            client,
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

    fn make_proxy_response(orig_resp: reqwest::Response) -> Response<Body> {
        let mut resp_builder = Response::builder().status(orig_resp.status());
        {
            let headers = resp_builder.headers_mut().unwrap();
            for (k, v) in orig_resp.headers() {
                if !HOP_BY_HOP_HEADERS.contains(k) {
                    headers.append(k, v.clone());
                }
            }
        }
        resp_builder
            .body(Body::from_stream(orig_resp.bytes_stream()))
            .unwrap()
    }

    async fn post_proxy(&self, path: &str, Json(payload): Json<Value>) -> Result<Response<Body>> {
        let orig_resp = self
            .client
            .post(self.endpoint(path))
            .json(&payload)
            .send()
            .await?;
        Ok(Self::make_proxy_response(orig_resp))
    }

    async fn get_proxy(&self, path: &str) -> Result<Response<Body>> {
        let orig_resp = self.client.get(self.endpoint(path)).send().await?;
        Ok(Self::make_proxy_response(orig_resp))
    }

    async fn chat_handler(
        State(state): State<Arc<LlmProxy>>,
        Json(payload): Json<Value>,
    ) -> Response<Body> {
        match state.post_proxy("/chat/completions", Json(payload)).await {
            Ok(resp) => resp,
            Err(e) => {
                event!(Level::ERROR, "streaming_aware_proxy failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error":{"message":e.to_string()}})).into_response(),
                )
                    .into_response()
            }
        }
    }

    async fn model_handler(State(state): State<Arc<LlmProxy>>) -> Response<Body> {
        match state.get_proxy("/models").await {
            Ok(resp) => resp,
            Err(e) => {
                event!(Level::ERROR, "model_handler failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error":{"message":e.to_string()}})).into_response(),
                )
                    .into_response()
            }
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

    let model_gateway = LlmProxy::builder()
        .base_url(&base_url)
        .maybe_api_key(api_key.as_deref())
        .build();

    event!(
        Level::INFO,
        "Listening on {bind_addr}; forwarding to {}",
        &model_gateway.base_url
    );

    let shared_model_gateway = Arc::new(model_gateway);

    let app = Router::new()
        .route("/v1/models", get(LlmProxy::model_handler))
        .route("/v1/chat/completions", post(LlmProxy::chat_handler))
        .with_state(shared_model_gateway);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    event!(Level::INFO, "Exiting...");

    Ok(())
}
