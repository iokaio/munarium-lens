// SPDX-License-Identifier: Apache-2.0

use std::future::Future;
use std::io;

use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

mod backend;

use backend::{EmbeddingBackend, RerankingBackend, UnavailableBackend};

const BODY_LIMIT: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RerankRequest {
    model: String,
    query: String,
    documents: Vec<String>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Embedding,
    Reranking,
}

#[derive(Serialize)]
struct Problem {
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stage: Option<Stage>,
}

fn problem(status: StatusCode, code: &'static str, stage: Option<Stage>) -> Response {
    (status, Json(Problem { code, stage })).into_response()
}

fn invalid_json(rejection: JsonRejection, stage: Stage) -> Response {
    let (status, code) = match rejection.status() {
        StatusCode::PAYLOAD_TOO_LARGE => (StatusCode::PAYLOAD_TOO_LARGE, "body_too_large"),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => {
            (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type")
        }
        _ => (StatusCode::BAD_REQUEST, "invalid_request"),
    };
    problem(status, code, Some(stage))
}

fn unavailable(stage: Stage) -> Response {
    problem(
        StatusCode::SERVICE_UNAVAILABLE,
        "inference_unavailable",
        Some(stage),
    )
}

async fn ready() -> Response {
    let backend = UnavailableBackend;
    if EmbeddingBackend::identity(&backend).is_ok() && RerankingBackend::identity(&backend).is_ok()
    {
        return StatusCode::NO_CONTENT.into_response();
    }
    problem(StatusCode::SERVICE_UNAVAILABLE, "backend_not_ready", None)
}

async fn embed(request: Result<Json<EmbedRequest>, JsonRejection>) -> Response {
    let stage = Stage::Embedding;
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return invalid_json(rejection, stage),
    };
    if request.model.trim().is_empty() || request.input.is_empty() {
        return problem(StatusCode::BAD_REQUEST, "invalid_request", Some(stage));
    }
    let backend = UnavailableBackend;
    match backend.embed(&request.model, &request.input) {
        Ok(vectors) => Json(vectors).into_response(),
        Err(_) => unavailable(stage),
    }
}

async fn rerank(request: Result<Json<RerankRequest>, JsonRejection>) -> Response {
    let stage = Stage::Reranking;
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return invalid_json(rejection, stage),
    };
    if request.model.trim().is_empty() || request.query.is_empty() || request.documents.is_empty() {
        return problem(StatusCode::BAD_REQUEST, "invalid_request", Some(stage));
    }
    let backend = UnavailableBackend;
    match backend.rerank(&request.model, &request.query, &request.documents) {
        Ok(results) => Json(results).into_response(),
        Err(_) => unavailable(stage),
    }
}

pub async fn serve(
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    if !listener.local_addr()?.ip().is_loopback() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "scaffold requires a loopback listener",
        ));
    }
    let app = Router::new()
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/readyz", get(ready))
        .route("/v1/embeddings", post(embed))
        .route("/v1/rerank", post(rerank))
        .layer(DefaultBodyLimit::max(BODY_LIMIT));
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}
