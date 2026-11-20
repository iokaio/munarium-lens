// SPDX-License-Identifier: Apache-2.0

//! The deterministic reference backend served over loopback HTTP.

mod common;

use std::net::SocketAddr;
use std::time::Duration;

use common::{Reply, request};
use munarium_lens::{Config, Limits, StageBackend};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const EMBED_MODEL: &str = "lens-reference/embedding-v1-16d";
const RERANK_MODEL: &str = "lens-reference/rerank-v1";

struct Running {
    addr: SocketAddr,
    stop: oneshot::Sender<()>,
    server: JoinHandle<std::io::Result<()>>,
}

impl Running {
    async fn start(config: Config) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, shutdown) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            munarium_lens::serve_with(listener, &config, async move {
                let _ = shutdown.await;
            })
            .await
        });
        Self { addr, stop, server }
    }

    async fn post(&self, path: &str, body: &Value) -> (u16, Value) {
        let reply = request(self.addr, "POST", path, Some(&body.to_string())).await;
        (reply.status, serde_json::from_str(&reply.body).unwrap())
    }

    async fn get(&self, path: &str) -> Reply {
        request(self.addr, "GET", path, None).await
    }

    async fn stop(self) {
        self.stop.send(()).unwrap();
        self.server.await.unwrap().unwrap();
    }
}

fn reference(limits: Limits) -> Config {
    Config {
        embedding: StageBackend::Reference,
        reranking: StageBackend::Reference,
        reference_dimensions: 16,
        limits,
        ..Config::default()
    }
}

fn small_limits() -> Limits {
    Limits {
        max_body_bytes: 4096,
        max_items: 3,
        max_input_tokens: 4,
        max_batch_tokens: 6,
        ..Limits::default()
    }
}

fn problem(code: &str, stage: &str) -> Value {
    json!({ "code": code, "stage": stage })
}

#[tokio::test]
async fn readiness_and_discovery_report_identity_dimensions_and_limits() {
    timeout(Duration::from_secs(5), async {
        let lens = Running::start(reference(small_limits())).await;
        let ready = lens.get("/readyz").await;
        assert_eq!(ready.status, 200);
        assert!(ready.head.contains("content-type: application/json"));
        let limits = json!({
            "max_body_bytes": 4096,
            "max_items": 3,
            "max_input_tokens": 4,
            "max_batch_tokens": 6,
        });
        assert_eq!(
            serde_json::from_str::<Value>(&ready.body).unwrap(),
            json!({
                "status": "ready",
                "stages": [
                    {
                        "stage": "embedding",
                        "model": EMBED_MODEL,
                        "backend": "reference",
                        "retrieval_model": false,
                        "dimensions": 16,
                        "manifest_sha256": null,
                        "limits": limits,
                    },
                    {
                        "stage": "reranking",
                        "model": RERANK_MODEL,
                        "backend": "reference",
                        "retrieval_model": false,
                        "manifest_sha256": null,
                        "limits": limits,
                    },
                ],
            })
        );
        let models = lens.get("/v1/models").await;
        assert_eq!(models.status, 200);
        assert_eq!(
            serde_json::from_str::<Value>(&models.body).unwrap(),
            json!({
                "object": "list",
                "data": [
                    {
                        "id": EMBED_MODEL,
                        "object": "model",
                        "owned_by": "munarium-lens",
                        "stage": "embedding",
                        "backend": "reference",
                        "retrieval_model": false,
                        "dimensions": 16,
                    },
                    {
                        "id": RERANK_MODEL,
                        "object": "model",
                        "owned_by": "munarium-lens",
                        "stage": "reranking",
                        "backend": "reference",
                        "retrieval_model": false,
                    },
                ],
            })
        );
        lens.stop().await;
    })
    .await
    .expect("timed out");
}

#[tokio::test]
async fn embeds_in_input_order_at_the_declared_dimension() {
    timeout(Duration::from_secs(5), async {
        let lens = Running::start(reference(Limits::default())).await;
        let inputs = ["An orchard grows fruit.", "", "an ORCHARD grows fruit"];
        let (status, body) = lens
            .post(
                "/v1/embeddings",
                &json!({ "model": EMBED_MODEL, "input": inputs }),
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(body["object"], "list");
        assert_eq!(body["model"], EMBED_MODEL);
        assert_eq!(
            body["usage"],
            json!({ "prompt_tokens": 8, "total_tokens": 8 })
        );
        let rows = body["data"].as_array().unwrap();
        assert_eq!(rows.len(), inputs.len());
        let mut vectors = Vec::new();
        for (expected, row) in rows.iter().enumerate() {
            assert_eq!(row["object"], "embedding");
            assert_eq!(row["index"], expected);
            let vector: Vec<f64> = row["embedding"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_f64().unwrap())
                .collect();
            assert_eq!(vector.len(), 16);
            let norm: f64 = vector.iter().map(|value| value * value).sum();
            assert!((norm - 1.0).abs() < 1e-5);
            vectors.push(vector);
        }
        // Case and punctuation do not change the reference tokens.
        assert_eq!(vectors[0], vectors[2]);
        assert_ne!(vectors[0], vectors[1]);
        // The same input gives the same vector on every call.
        let (_, again) = lens
            .post(
                "/v1/embeddings",
                &json!({ "model": EMBED_MODEL, "input": [inputs[0]] }),
            )
            .await;
        assert_eq!(again["data"][0]["embedding"], rows[0]["embedding"]);
        lens.stop().await;
    })
    .await
    .expect("timed out");
}

#[tokio::test]
async fn reranks_into_a_complete_permutation_with_stable_ties() {
    timeout(Duration::from_secs(5), async {
        let lens = Running::start(reference(Limits::default())).await;
        let (status, body) = lens
            .post(
                "/v1/rerank",
                &json!({
                    "model": RERANK_MODEL,
                    "query": "orchard fruit",
                    "documents": [
                        "A library lends books.",
                        "An orchard grows fruit.",
                        "Fruit is sold at market.",
                        "An orchard in spring.",
                        "Shelves of books.",
                    ],
                }),
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(
            body,
            json!({
                "model": RERANK_MODEL,
                "results": [
                    { "index": 1, "relevance_score": 1.0 },
                    { "index": 2, "relevance_score": 0.5 },
                    { "index": 3, "relevance_score": 0.5 },
                    { "index": 0, "relevance_score": 0.0 },
                    { "index": 4, "relevance_score": 0.0 },
                ],
                // Each pair counts the 2 query tokens plus its document's.
                "usage": { "prompt_tokens": 30, "total_tokens": 30 },
            })
        );
        lens.stop().await;
    })
    .await
    .expect("timed out");
}

#[tokio::test]
async fn refuses_bound_violations_before_inference_in_a_fixed_order() {
    timeout(Duration::from_secs(5), async {
        let lens = Running::start(reference(small_limits())).await;
        let cases = [
            // Identity precedes every bound, so a wrong model with too many
            // items reports model_not_found.
            (
                json!({ "model": "other", "input": ["a", "b", "c", "d"] }),
                json!({ "model": "other", "query": "q", "documents": ["a", "b", "c", "d"] }),
                404,
                "model_not_found",
            ),
            // The item count precedes tokenization.
            (
                json!({ "model": EMBED_MODEL, "input": ["a", "b", "c", "one two three four five"] }),
                json!({ "model": RERANK_MODEL, "query": "q", "documents": ["a", "b", "c", "one two three four five"] }),
                413,
                "too_many_items",
            ),
            // One overlong item refuses the whole request.
            (
                json!({ "model": EMBED_MODEL, "input": ["a", "one two three four five"] }),
                json!({ "model": RERANK_MODEL, "query": "q", "documents": ["a", "one two three four"] }),
                413,
                "input_too_long",
            ),
            (
                json!({ "model": EMBED_MODEL, "input": ["a b c", "d e f", "g"] }),
                json!({ "model": RERANK_MODEL, "query": "q", "documents": ["a b", "c d", "e"] }),
                413,
                "batch_too_large",
            ),
            (
                json!({ "model": EMBED_MODEL, "input": "not a list" }),
                json!({ "model": RERANK_MODEL, "query": "q", "documents": ["a"], "top_n": 1 }),
                400,
                "invalid_request",
            ),
            (
                json!({ "model": EMBED_MODEL, "input": ["a"], "dimensions": 8 }),
                json!({ "model": RERANK_MODEL, "query": "", "documents": ["a"] }),
                400,
                "invalid_request",
            ),
        ];
        for (embed, rerank, status, code) in cases {
            assert_eq!(
                lens.post("/v1/embeddings", &embed).await,
                (status, problem(code, "embedding"))
            );
            assert_eq!(
                lens.post("/v1/rerank", &rerank).await,
                (status, problem(code, "reranking"))
            );
        }
        // The largest accepted request still succeeds.
        let (status, _) = lens
            .post(
                "/v1/embeddings",
                &json!({ "model": EMBED_MODEL, "input": ["a b c d", "e", "f"] }),
            )
            .await;
        assert_eq!(status, 200);
        let oversized = format!(r#"{{"model":"{EMBED_MODEL}","input":["{}"]}}"#, "a".repeat(4096));
        let reply = request(lens.addr, "POST", "/v1/embeddings", Some(&oversized)).await;
        assert_eq!(reply.status, 413);
        assert_eq!(
            serde_json::from_str::<Value>(&reply.body).unwrap(),
            problem("body_too_large", "embedding")
        );
        lens.stop().await;
    })
    .await
    .expect("timed out");
}

#[tokio::test]
async fn documented_walkthrough_fixtures_succeed_with_default_settings() {
    timeout(Duration::from_secs(5), async {
        let lens = Running::start(Config {
            embedding: StageBackend::Reference,
            reranking: StageBackend::Reference,
            ..Config::default()
        })
        .await;
        let embed: Value =
            serde_json::from_str(include_str!("fixtures/reference-embed.json")).unwrap();
        let (status, body) = lens.post("/v1/embeddings", &embed).await;
        assert_eq!(status, 200);
        assert_eq!(body["data"][1]["embedding"].as_array().unwrap().len(), 1024);
        let rerank: Value =
            serde_json::from_str(include_str!("fixtures/reference-rerank.json")).unwrap();
        assert_eq!(
            lens.post("/v1/rerank", &rerank).await,
            (
                200,
                json!({
                    "model": RERANK_MODEL,
                    "results": [
                        { "index": 1, "relevance_score": 0.6 },
                        { "index": 0, "relevance_score": 0.0 },
                    ],
                    "usage": { "prompt_tokens": 18, "total_tokens": 18 },
                })
            )
        );
        lens.stop().await;
    })
    .await
    .expect("timed out");
}

#[tokio::test]
async fn serves_one_stage_and_refuses_the_disabled_one() {
    timeout(Duration::from_secs(5), async {
        let lens = Running::start(Config {
            reranking: StageBackend::Disabled,
            ..reference(Limits::default())
        })
        .await;
        let ready: Value = serde_json::from_str(&lens.get("/readyz").await.body).unwrap();
        assert_eq!(ready["stages"].as_array().unwrap().len(), 1);
        assert_eq!(ready["stages"][0]["stage"], "embedding");
        assert_eq!(
            lens.post(
                "/v1/rerank",
                &json!({ "model": RERANK_MODEL, "query": "q", "documents": ["a"] })
            )
            .await,
            (503, problem("inference_unavailable", "reranking"))
        );
        lens.stop().await;
    })
    .await
    .expect("timed out");
}
