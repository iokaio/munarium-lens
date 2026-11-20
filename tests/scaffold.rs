// SPDX-License-Identifier: Apache-2.0

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::time::timeout;

async fn request(addr: SocketAddr, method: &str, path: &str, json: Option<&str>) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    if let Some(body) = json {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    if let Some(body) = json {
        stream.write_all(body.as_bytes()).await.unwrap();
    }
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8(response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, body.to_owned())
}

#[tokio::test]
async fn serves_synthetic_requests_and_shuts_down() {
    timeout(Duration::from_secs(5), async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (stop, shutdown) = oneshot::channel();
        let server = tokio::spawn(munarium_lens::serve(listener, async move {
            shutdown.await.unwrap();
        }));

        assert_eq!(
            request(addr, "GET", "/healthz", None).await,
            (204, String::new())
        );
        assert_eq!(
            request(addr, "GET", "/readyz", None).await,
            (503, r#"{"code":"backend_not_ready"}"#.to_owned())
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
            let (status, body) = request(addr, "POST", path, Some(data)).await;
            assert_eq!(status, 503);
            assert_eq!(
                body,
                format!(r#"{{"code":"inference_unavailable","stage":"{stage}"}}"#)
            );
        }
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
            (
                Some(" ".repeat(16 * 1024 + 1).as_str()),
                413,
                "body_too_large",
            ),
        ] {
            let (status, body) = request(addr, "POST", "/v1/embeddings", bad_request).await;
            assert_eq!(status, expected_status);
            assert_eq!(
                body,
                format!(r#"{{"code":"{expected_code}","stage":"embedding"}}"#)
            );
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
