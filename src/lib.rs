// SPDX-License-Identifier: Apache-2.0

//! Munarium Lens HTTP service: readiness, model discovery, embeddings and
//! reranking behind per-stage backends. See `docs/design/endpoint-profile.md`.

use std::future::Future;
use std::io;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::rejection::JsonRejection;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

mod admission;
mod backend;
mod config;
#[cfg(feature = "qwen3")]
mod manifest;
#[cfg(feature = "qwen3")]
mod qwen3;
mod reference;

use admission::Admission;
use backend::{BackendError, EmbeddingBackend, RerankHit, RerankingBackend, ServedIdentity};
pub use config::{Config, ConfigError, Limits, StageArtifacts, StageBackend};

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

fn overloaded(stage: Stage) -> Response {
    let mut response = problem(StatusCode::SERVICE_UNAVAILABLE, "overloaded", Some(stage));
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, header::HeaderValue::from_static("1"));
    response
}

/// A stage's state after startup. Only `Ready` stages accept work.
enum StageState<B: ?Sized> {
    Disabled,
    /// Configured, but loading or the startup probe failed.
    Failed,
    Ready {
        backend: Arc<B>,
        admission: Admission,
    },
}

impl<B: ?Sized> StageState<B> {
    fn label(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Failed => "failed",
            Self::Ready { .. } => "ready",
        }
    }
}

struct AppState {
    embedding: StageState<dyn EmbeddingBackend>,
    reranking: StageState<dyn RerankingBackend>,
    limits: Limits,
}

/// Content-free startup probe of a loaded embedding backend.
fn probe_embedding(backend: &dyn EmbeddingBackend) -> bool {
    let Some(dimensions) = backend.identity().dimensions else {
        return false;
    };
    let inputs = ["Lens readiness probe".to_owned(), String::new()];
    matches!(backend.embed(&inputs), Ok(vectors) if valid_vectors(&vectors, inputs.len(), dimensions))
}

/// Content-free startup probe of a loaded reranking backend.
fn probe_reranking(backend: &dyn RerankingBackend) -> bool {
    let documents = ["readiness probe".to_owned(), "unrelated".to_owned()];
    matches!(backend.score("readiness probe", &documents), Ok(scores) if valid_scores(&scores, documents.len()))
}

fn valid_vectors(vectors: &[Vec<f32>], count: usize, dimensions: usize) -> bool {
    vectors.len() == count
        && vectors.iter().all(|vector| {
            vector.len() == dimensions && vector.iter().all(|value| value.is_finite())
        })
}

fn valid_scores(scores: &[f32], count: usize) -> bool {
    scores.len() == count && scores.iter().all(|score| score.is_finite())
}

/// Log a content-free load failure category for a stage.
#[cfg(feature = "qwen3")]
fn load_failed(stage: &str, error: manifest::ArtifactError) {
    eprintln!("Lens {stage} stage did not load: {error}");
}

#[cfg(feature = "qwen3")]
fn load_qwen3_embedding(config: &Config) -> Option<Arc<dyn EmbeddingBackend>> {
    let artifacts = config.embedding_artifacts.as_ref()?;
    match qwen3::Qwen3Embedding::load(
        &artifacts.directory,
        &artifacts.manifest_sha256,
        config.limits.max_input_tokens,
    ) {
        Ok(backend) => Some(Arc::new(backend)),
        Err(error) => {
            load_failed("embedding", error);
            None
        }
    }
}

#[cfg(feature = "qwen3")]
fn load_qwen3_reranking(config: &Config) -> Option<Arc<dyn RerankingBackend>> {
    let artifacts = config.reranking_artifacts.as_ref()?;
    match qwen3::Qwen3Reranking::load(
        &artifacts.directory,
        &artifacts.manifest_sha256,
        config.limits.max_input_tokens,
    ) {
        Ok(backend) => Some(Arc::new(backend)),
        Err(error) => {
            load_failed("reranking", error);
            None
        }
    }
}

impl AppState {
    /// Load and probe each configured stage. Model loading reads and hashes
    /// the artifacts, so call this off the async runtime.
    fn from_config(config: &Config) -> Self {
        let limits = config.limits.clone();
        let admission =
            || Admission::new(limits.max_in_flight, limits.max_queue, limits.queue_timeout);
        let embedding: Option<Arc<dyn EmbeddingBackend>> = match config.embedding {
            StageBackend::Disabled => None,
            StageBackend::Reference => Some(Arc::new(reference::ReferenceEmbedding::new(
                config.reference_dimensions,
            ))),
            #[cfg(feature = "qwen3")]
            StageBackend::Qwen3 => load_qwen3_embedding(config),
        };
        let embedding = match (config.embedding, embedding) {
            (StageBackend::Disabled, _) => StageState::Disabled,
            (_, Some(backend)) if probe_embedding(backend.as_ref()) => StageState::Ready {
                backend,
                admission: admission(),
            },
            _ => StageState::Failed,
        };
        let reranking: Option<Arc<dyn RerankingBackend>> = match config.reranking {
            StageBackend::Disabled => None,
            StageBackend::Reference => Some(Arc::new(reference::ReferenceReranking::new())),
            #[cfg(feature = "qwen3")]
            StageBackend::Qwen3 => load_qwen3_reranking(config),
        };
        let reranking = match (config.reranking, reranking) {
            (StageBackend::Disabled, _) => StageState::Disabled,
            (_, Some(backend)) if probe_reranking(backend.as_ref()) => StageState::Ready {
                backend,
                admission: admission(),
            },
            _ => StageState::Failed,
        };
        Self {
            embedding,
            reranking,
            limits,
        }
    }

    fn identities(&self) -> Vec<(Stage, &ServedIdentity)> {
        let mut found = Vec::new();
        if let StageState::Ready { backend, .. } = &self.embedding {
            found.push((Stage::Embedding, backend.identity()));
        }
        if let StageState::Ready { backend, .. } = &self.reranking {
            found.push((Stage::Reranking, backend.identity()));
        }
        found
    }
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Serialize)]
struct StageLimits {
    max_body_bytes: usize,
    max_items: usize,
    max_input_tokens: usize,
    max_batch_tokens: usize,
}

#[derive(Serialize)]
struct StageReport<'a> {
    stage: Stage,
    model: &'a str,
    backend: &'static str,
    retrieval_model: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
    manifest_sha256: Option<String>,
    limits: StageLimits,
}

#[derive(Serialize)]
struct Readiness<'a> {
    status: &'static str,
    stages: Vec<StageReport<'a>>,
}

async fn ready(State(state): State<Arc<AppState>>) -> Response {
    let failed = matches!(state.embedding, StageState::Failed)
        || matches!(state.reranking, StageState::Failed);
    let identities = state.identities();
    if failed || identities.is_empty() {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "backend_not_ready", None);
    }
    let limits = &state.limits;
    let stages = identities
        .into_iter()
        .map(|(stage, identity)| StageReport {
            stage,
            model: &identity.model,
            backend: identity.backend,
            retrieval_model: identity.retrieval_model,
            dimensions: identity.dimensions,
            manifest_sha256: identity.manifest_sha256.as_ref().map(hex),
            limits: StageLimits {
                max_body_bytes: limits.max_body_bytes,
                max_items: limits.max_items,
                max_input_tokens: limits.max_input_tokens,
                max_batch_tokens: limits.max_batch_tokens,
            },
        })
        .collect();
    Json(Readiness {
        status: "ready",
        stages,
    })
    .into_response()
}

#[derive(Serialize)]
struct ModelCard<'a> {
    id: &'a str,
    object: &'static str,
    owned_by: &'static str,
    stage: Stage,
    backend: &'static str,
    retrieval_model: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<usize>,
}

#[derive(Serialize)]
struct ModelList<'a> {
    object: &'static str,
    data: Vec<ModelCard<'a>>,
}

async fn models(State(state): State<Arc<AppState>>) -> Response {
    let data = state
        .identities()
        .into_iter()
        .map(|(stage, identity)| ModelCard {
            id: &identity.model,
            object: "model",
            owned_by: "munarium-lens",
            stage,
            backend: identity.backend,
            retrieval_model: identity.retrieval_model,
            dimensions: identity.dimensions,
        })
        .collect();
    Json(ModelList {
        object: "list",
        data,
    })
    .into_response()
}

#[derive(Serialize)]
struct Usage {
    prompt_tokens: usize,
    total_tokens: usize,
}

impl Usage {
    fn tokens(count: usize) -> Self {
        Self {
            prompt_tokens: count,
            total_tokens: count,
        }
    }
}

#[derive(Serialize)]
struct EmbeddingRow {
    object: &'static str,
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Serialize)]
struct EmbeddingResponse<'a> {
    object: &'static str,
    model: &'a str,
    data: Vec<EmbeddingRow>,
    usage: Usage,
}

#[derive(Serialize)]
struct RerankResponse<'a> {
    model: &'a str,
    results: Vec<RerankHit>,
    usage: Usage,
}

/// A content-free refusal before its stage is attached.
type Refusal = (StatusCode, &'static str);

const INFERENCE_FAILED: Refusal = (StatusCode::INTERNAL_SERVER_ERROR, "inference_failed");

fn refuse((status, code): Refusal, stage: Stage) -> Response {
    problem(status, code, Some(stage))
}

/// Check token bounds over per-item counts. Returns the total on success.
fn check_tokens(counts: &[usize], limits: &Limits) -> Result<usize, Refusal> {
    if counts.iter().any(|&count| count > limits.max_input_tokens) {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "input_too_long"));
    }
    let total = counts.iter().sum::<usize>();
    if total > limits.max_batch_tokens {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "batch_too_large"));
    }
    Ok(total)
}

/// Map a blocking task outcome to a result or a content-free refusal.
fn finish<T>(
    outcome: Result<Result<T, BackendError>, tokio::task::JoinError>,
) -> Result<T, Refusal> {
    match outcome {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(BackendError::Unavailable)) => {
            Err((StatusCode::SERVICE_UNAVAILABLE, "inference_unavailable"))
        }
        Err(_) => Err(INFERENCE_FAILED),
    }
}

async fn embed(
    State(state): State<Arc<AppState>>,
    request: Result<Json<EmbedRequest>, JsonRejection>,
) -> Response {
    let stage = Stage::Embedding;
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return invalid_json(rejection, stage),
    };
    if request.model.trim().is_empty() || request.input.is_empty() {
        return problem(StatusCode::BAD_REQUEST, "invalid_request", Some(stage));
    }
    let StageState::Ready { backend, admission } = &state.embedding else {
        return problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "inference_unavailable",
            Some(stage),
        );
    };
    let identity = backend.identity();
    if request.model != identity.model {
        return problem(StatusCode::NOT_FOUND, "model_not_found", Some(stage));
    }
    if request.input.len() > state.limits.max_items {
        return problem(StatusCode::PAYLOAD_TOO_LARGE, "too_many_items", Some(stage));
    }
    let counts: Vec<usize> = request
        .input
        .iter()
        .map(|input| backend.count_tokens(input))
        .collect();
    let total = match check_tokens(&counts, &state.limits) {
        Ok(total) => total,
        Err(refusal) => return refuse(refusal, stage),
    };
    let Ok(permit) = admission.admit().await else {
        return overloaded(stage);
    };
    let count = request.input.len();
    let worker = Arc::clone(backend);
    let outcome = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        worker.embed(&request.input)
    })
    .await;
    let vectors = match finish(outcome) {
        Ok(vectors) => vectors,
        Err(refusal) => return refuse(refusal, stage),
    };
    let Some(dimensions) = identity.dimensions else {
        return refuse(INFERENCE_FAILED, stage);
    };
    if !valid_vectors(&vectors, count, dimensions) {
        return refuse(INFERENCE_FAILED, stage);
    }
    Json(EmbeddingResponse {
        object: "list",
        model: &identity.model,
        data: vectors
            .into_iter()
            .enumerate()
            .map(|(index, embedding)| EmbeddingRow {
                object: "embedding",
                index,
                embedding,
            })
            .collect(),
        usage: Usage::tokens(total),
    })
    .into_response()
}

async fn rerank(
    State(state): State<Arc<AppState>>,
    request: Result<Json<RerankRequest>, JsonRejection>,
) -> Response {
    let stage = Stage::Reranking;
    let Json(request) = match request {
        Ok(request) => request,
        Err(rejection) => return invalid_json(rejection, stage),
    };
    if request.model.trim().is_empty() || request.query.is_empty() || request.documents.is_empty() {
        return problem(StatusCode::BAD_REQUEST, "invalid_request", Some(stage));
    }
    let StageState::Ready { backend, admission } = &state.reranking else {
        return problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "inference_unavailable",
            Some(stage),
        );
    };
    let identity = backend.identity();
    if request.model != identity.model {
        return problem(StatusCode::NOT_FOUND, "model_not_found", Some(stage));
    }
    if request.documents.len() > state.limits.max_items {
        return problem(StatusCode::PAYLOAD_TOO_LARGE, "too_many_items", Some(stage));
    }
    let counts: Vec<usize> = request
        .documents
        .iter()
        .map(|document| backend.count_pair_tokens(&request.query, document))
        .collect();
    let total = match check_tokens(&counts, &state.limits) {
        Ok(total) => total,
        Err(refusal) => return refuse(refusal, stage),
    };
    let Ok(permit) = admission.admit().await else {
        return overloaded(stage);
    };
    let count = request.documents.len();
    let worker = Arc::clone(backend);
    let outcome = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        worker.score(&request.query, &request.documents)
    })
    .await;
    let scores = match finish(outcome) {
        Ok(scores) => scores,
        Err(refusal) => return refuse(refusal, stage),
    };
    if !valid_scores(&scores, count) {
        return refuse(INFERENCE_FAILED, stage);
    }
    Json(RerankResponse {
        model: &identity.model,
        results: backend::rank(&scores),
        usage: Usage::tokens(total),
    })
    .into_response()
}

fn router(state: Arc<AppState>) -> Router {
    let body_limit = state.limits.max_body_bytes;
    Router::new()
        .route("/healthz", get(|| async { StatusCode::NO_CONTENT }))
        .route("/readyz", get(ready))
        .route("/v1/models", get(models))
        .route("/v1/embeddings", post(embed))
        .route("/v1/rerank", post(rerank))
        .layer(DefaultBodyLimit::max(body_limit))
        .with_state(state)
}

/// Serve the scaffold configuration: loopback only, both stages disabled.
pub async fn serve(
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    serve_with(listener, &Config::default(), shutdown).await
}

/// Load and probe the configured stages, then serve until `shutdown`
/// completes. Accepted requests finish before this returns. The listener must
/// be on loopback unless the configuration explicitly allows otherwise; its
/// address takes precedence over `config.listen`.
pub async fn serve_with(
    listener: TcpListener,
    config: &Config,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    let address = listener.local_addr()?;
    if !address.ip().is_loopback() && !config.allow_unauthenticated_non_loopback {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a non-loopback listener requires LENS_ALLOW_UNAUTHENTICATED_NON_LOOPBACK=1",
        ));
    }
    eprintln!("Lens listening on {address}");
    let owned = config.clone();
    let state = tokio::task::spawn_blocking(move || AppState::from_config(&owned))
        .await
        .map_err(io::Error::other)?;
    let state = Arc::new(state);
    for (stage, state) in [
        ("embedding", state.embedding.label()),
        ("reranking", state.reranking.label()),
    ] {
        eprintln!("Lens {stage} stage: {state}");
    }
    for (_, identity) in state.identities() {
        if !identity.retrieval_model {
            eprintln!(
                "Lens serves {} from the {} backend, which is not a retrieval model",
                identity.model, identity.backend
            );
        }
    }
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use std::time::Duration;

    struct Faulty {
        identity: ServedIdentity,
        rows: usize,
        value: f32,
    }

    impl EmbeddingBackend for Faulty {
        fn identity(&self) -> &ServedIdentity {
            &self.identity
        }
        fn count_tokens(&self, _: &str) -> usize {
            1
        }
        fn embed(&self, _: &[String]) -> Result<Vec<Vec<f32>>, BackendError> {
            Ok(vec![vec![self.value; 2]; self.rows])
        }
    }

    impl RerankingBackend for Faulty {
        fn identity(&self) -> &ServedIdentity {
            &self.identity
        }
        fn count_pair_tokens(&self, _: &str, _: &str) -> usize {
            1
        }
        fn score(&self, _: &str, _: &[String]) -> Result<Vec<f32>, BackendError> {
            if self.rows == usize::MAX {
                return Err(BackendError::Unavailable);
            }
            Ok(vec![self.value; self.rows])
        }
    }

    fn faulty(rows: usize, value: f32, max_in_flight: usize) -> Arc<AppState> {
        let backend = Arc::new(Faulty {
            identity: ServedIdentity {
                model: "fixture-model".to_owned(),
                backend: "test",
                retrieval_model: false,
                dimensions: Some(2),
                manifest_sha256: Some([0xab; 32]),
            },
            rows,
            value,
        });
        let admission = || Admission::new(max_in_flight, 0, Duration::from_millis(10));
        Arc::new(AppState {
            embedding: StageState::Ready {
                backend: backend.clone(),
                admission: admission(),
            },
            reranking: StageState::Ready {
                backend,
                admission: admission(),
            },
            limits: Limits::default(),
        })
    }

    async fn parts(response: Response) -> (StatusCode, Option<String>, String) {
        let status = response.status();
        let retry = response
            .headers()
            .get(header::RETRY_AFTER)
            .map(|value| value.to_str().unwrap().to_owned());
        let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, retry, String::from_utf8(body.to_vec()).unwrap())
    }

    fn embed_request() -> Result<Json<EmbedRequest>, JsonRejection> {
        Ok(Json(EmbedRequest {
            model: "fixture-model".to_owned(),
            input: vec!["synthetic".to_owned(), "inputs".to_owned()],
        }))
    }

    fn rerank_request() -> Result<Json<RerankRequest>, JsonRejection> {
        Ok(Json(RerankRequest {
            model: "fixture-model".to_owned(),
            query: "synthetic".to_owned(),
            documents: vec!["a".to_owned(), "b".to_owned()],
        }))
    }

    #[tokio::test]
    async fn refuses_backend_output_with_wrong_count_or_nonfinite_values() {
        for state in [faulty(1, 0.5, 1), faulty(2, f32::NAN, 1)] {
            let (status, _, body) = parts(embed(State(state.clone()), embed_request()).await).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body, r#"{"code":"inference_failed","stage":"embedding"}"#);
            let (status, _, body) = parts(rerank(State(state), rerank_request()).await).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body, r#"{"code":"inference_failed","stage":"reranking"}"#);
        }
        let (status, _, body) = parts(embed(State(faulty(2, 0.5, 1)), embed_request()).await).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(r#""index":1"#));
    }

    #[tokio::test]
    async fn maps_backend_unavailability_to_a_typed_refusal() {
        let state = faulty(usize::MAX, 0.5, 1);
        let (status, _, body) = parts(rerank(State(state), rerank_request()).await).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            body,
            r#"{"code":"inference_unavailable","stage":"reranking"}"#
        );
    }

    #[tokio::test]
    async fn refuses_overload_before_dispatch_with_retry_guidance() {
        let state = faulty(2, 0.5, 1);
        let StageState::Ready { admission, .. } = &state.embedding else {
            unreachable!();
        };
        let held = admission.admit().await.unwrap();
        let (status, retry, body) = parts(embed(State(state.clone()), embed_request()).await).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(retry.as_deref(), Some("1"));
        assert_eq!(body, r#"{"code":"overloaded","stage":"embedding"}"#);
        // The reranking stage has its own capacity.
        let (status, _, _) = parts(rerank(State(state.clone()), rerank_request()).await).await;
        assert_eq!(status, StatusCode::OK);
        drop(held);
        let (status, _, _) = parts(embed(State(state), embed_request()).await).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn reports_a_manifest_digest_when_the_backend_has_one() {
        let (status, _, body) = parts(ready(State(faulty(2, 0.5, 1))).await).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains(&format!(r#""manifest_sha256":"{}""#, "ab".repeat(32))));
    }

    #[test]
    fn startup_probes_reject_invalid_backends() {
        let state = faulty(1, 0.5, 1);
        let StageState::Ready { backend, .. } = &state.embedding else {
            unreachable!();
        };
        assert!(!probe_embedding(backend.as_ref()));
        let nan = faulty(2, f32::INFINITY, 1);
        let StageState::Ready { backend, .. } = &nan.reranking else {
            unreachable!();
        };
        assert!(!probe_reranking(backend.as_ref()));
        let good = faulty(2, 0.5, 1);
        let StageState::Ready { backend, .. } = &good.embedding else {
            unreachable!();
        };
        assert!(probe_embedding(backend.as_ref()));
    }
}
