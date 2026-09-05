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
use tracing::{Level, event, instrument};

#[derive(Debug, Clone, Parser)]
#[command(name = "simple-llm-proxy")]
struct Args {
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
    #[arg(long, default_value = "3000")]
    port: u16,
    #[arg(long, default_value = "http://localhost:8080/v1")]
    base_url: String,
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

    #[instrument]
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

    #[instrument]
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
                Json(json!({"error": e.to_string()})).into_response(),
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
                Json(json!({"error": e.to_string()})).into_response(),
            )
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let bind_addr = format!("{}:{}", args.host, args.port);

    let api_key = match args.api_key {
        Some(val) => Some(val),
        None => match args.api_key_env {
            Some(var) => Some(std::env::var(var)?),
            None => None,
        },
    };

    let model_gateway = ModelGateway::new(args.base_url.as_str(), api_key.as_deref());

    let shared_model_gateway = Arc::new(model_gateway);

    let app = Router::new()
        .route("/v1/models", get(model_handler))
        .route("/v1/chat/completions", post(chat_handler))
        .with_state(shared_model_gateway);

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
