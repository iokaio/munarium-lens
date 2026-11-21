// SPDX-License-Identifier: Apache-2.0

//! The default configuration: loopback only, both stages disabled.

mod common;

use std::time::Duration;

use common::{request, request_with_headers};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;

#[tokio::test]
async fn serves_synthetic_requests_and_shuts_down() {
    timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, shutdown) = oneshot::channel();
        let server = tokio::spawn(munarium_lens::serve(listener, async move {
            shutdown.await.unwrap();
        }));

        let reply = request(addr, "GET", "/healthz", None).await;
        assert_eq!((reply.status, reply.body.as_str()), (204, ""));
        let reply = request(addr, "GET", "/readyz", None).await;
        assert_eq!(
            (reply.status, reply.body.as_str()),
            (503, r#"{"code":"backend_not_ready"}"#)
        );
        assert!(reply.head.contains("content-type: application/json"));
        let reply = request(addr, "GET", "/v1/models", None).await;
        assert_eq!(
            (reply.status, reply.body.as_str()),
            (200, r#"{"object":"list","data":[]}"#)
        );
        for (path, data, stage) in [
            (
                "/v1/embeddings",
                include_str!("fixtures/embed.json"),
                "embedding",
            ),
            (
                "/v1/rerank",
                include_str!("fixtures/rerank.json"),
                "reranking",
            ),
        ] {
            let reply = request(addr, "POST", path, Some(data)).await;
            assert_eq!(reply.status, 503);
            assert_eq!(
                reply.body,
                format!(r#"{{"code":"inference_unavailable","stage":"{stage}"}}"#)
            );
        }
        let limit = munarium_lens::Limits::default().max_body_bytes;
        for (bad_request, expected_status, expected_code) in [
            (None, 415, "unsupported_media_type"),
            (
                Some(r#"{"input":["private request text"]}"#),
                400,
                "invalid_request",
            ),
            (
                Some(r#"{"model":"fixture-model","input":[]}"#),
                400,
                "invalid_request",
            ),
            (Some(" "), 400, "invalid_request"),
            (Some(" ".repeat(limit + 1).as_str()), 413, "body_too_large"),
        ] {
            let reply = request(addr, "POST", "/v1/embeddings", bad_request).await;
            assert_eq!(reply.status, expected_status);
            assert_eq!(
                reply.body,
                format!(r#"{{"code":"{expected_code}","stage":"embedding"}}"#)
            );
            assert!(!reply.body.contains("private"));
        }
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
        assert!(TcpStream::connect(addr).await.is_err());
    })
    .await
    .expect("scaffold lifecycle timed out");
}

#[tokio::test]
async fn rejects_a_non_loopback_listener() {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let err = munarium_lens::serve(listener, std::future::pending())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
}

#[tokio::test]
async fn authenticates_before_body_and_model_access() {
    let secret = "01".repeat(32);
    let credential_path =
        std::env::temp_dir().join(format!("lens-bearer-fixture-{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&credential_path)
        .unwrap();
    std::io::Write::write_all(&mut file, format!("{secret}\r\n").as_bytes()).unwrap();
    drop(file);
    let config = munarium_lens::Config {
        service_bearer_file: Some(credential_path.clone()),
        embedding: munarium_lens::StageBackend::Reference,
        ..munarium_lens::Config::default()
    };
    let result = timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
        let addr = ([127, 0, 0, 1], listener.local_addr().unwrap().port()).into();
        let (stop, shutdown) = oneshot::channel();
        let server = tokio::spawn(async move {
            munarium_lens::serve_with(listener, &config, async move {
                let _ = shutdown.await;
            })
            .await
        });
        assert_eq!(request(addr, "GET", "/healthz", None).await.status, 204);
        for (method, path, body) in [
            ("GET", "/readyz", None),
            ("GET", "/v1/models", None),
            ("POST", "/v1/embeddings", Some("{not json")),
            ("POST", "/v1/rerank", Some("{not json")),
        ] {
            let reply = request(addr, method, path, body).await;
            assert_eq!(reply.status, 401);
            assert_eq!(reply.body, r#"{"code":"service_auth_required"}"#);
            assert!(reply.head.contains("www-authenticate: bearer"));
        }
        let correct = format!("Bearer {secret}");
        let wrong = format!("Bearer {}", "10".repeat(32));
        for headers in [
            vec![("Authorization", wrong.as_str())],
            vec![
                ("Authorization", correct.as_str()),
                ("Authorization", correct.as_str()),
            ],
        ] {
            assert_eq!(
                request_with_headers(addr, "GET", "/v1/models", None, &headers)
                    .await
                    .status,
                401
            );
        }
        let headers = [("Authorization", correct.as_str())];
        assert_eq!(
            request_with_headers(addr, "GET", "/readyz", None, &headers)
                .await
                .status,
            200
        );
        let reply = request_with_headers(addr, "GET", "/v1/models", None, &headers).await;
        assert_eq!(reply.status, 200);
        assert!(reply.body.contains("lens-reference/embedding-v1-1024d"));
        let reply =
            request_with_headers(addr, "POST", "/v1/embeddings", Some("{not json"), &headers).await;
        assert_eq!(reply.status, 400);
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
    })
    .await;
    std::fs::remove_file(&credential_path).unwrap();
    result.expect("authenticated service lifecycle timed out");
}
