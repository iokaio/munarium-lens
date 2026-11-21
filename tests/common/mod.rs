// SPDX-License-Identifier: Apache-2.0

//! Minimal HTTP/1.1 client for the offline lifecycle tests.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub struct Reply {
    pub status: u16,
    pub head: String,
    pub body: String,
}

pub async fn request(addr: SocketAddr, method: &str, path: &str, json: Option<&str>) -> Reply {
    request_with_headers(addr, method, path, json, &[]).await
}

pub async fn request_with_headers(
    addr: SocketAddr,
    method: &str,
    path: &str,
    json: Option<&str>,
    headers: &[(&str, &str)],
) -> Reply {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");
    if let Some(body) = json {
        head.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
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
    Reply {
        status,
        head: head.to_ascii_lowercase(),
        body: body.to_owned(),
    }
}
