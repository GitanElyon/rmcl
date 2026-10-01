// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// Tokio time is paused in retrying tests so the production backoff remains
// covered without adding wall-clock delay to the suite.

use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use serde::Deserialize;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use rmcl::net::{HttpClient, download_file};

#[derive(Debug, Deserialize)]
struct ApiResponse {
    ok: bool,
}

fn client_without_timeout() -> HttpClient {
    reqwest::Client::builder().build().unwrap().into()
}

#[tokio::test(start_paused = true)]
async fn get_json_retries_5xx_then_succeeds() {
    let server = MockServer::start().await;
    let attempts = Arc::new(AtomicUsize::new(0));

    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(move |_: &wiremock::Request| {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"ok": true}))
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let url = format!("{}/api", server.uri());
    let result: ApiResponse = client_without_timeout().get_json(&url).await.unwrap();
    assert!(result.ok);
}

#[tokio::test(start_paused = true)]
async fn get_json_retries_rate_limit() {
    let server = MockServer::start().await;
    let attempts = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/limited"))
        .respond_with(move |_: &wiremock::Request| {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(429)
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"ok": true}))
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let result: ApiResponse = client_without_timeout()
        .get_json(&format!("{}/limited", server.uri()))
        .await
        .unwrap();
    assert!(result.ok);
}

#[tokio::test]
async fn get_json_fails_fast_on_4xx() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/api", server.uri());
    let err = HttpClient::new()
        .get_json::<ApiResponse>(&url)
        .await
        .unwrap_err();
    assert!(
        format!("{err:?}").contains("404"),
        "expected 404 in error, got: {err:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn get_json_gives_up_after_max_retries() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api"))
        .respond_with(ResponseTemplate::new(503))
        .expect(4)
        .mount(&server)
        .await;

    let url = format!("{}/api", server.uri());
    let err = client_without_timeout()
        .get_json::<ApiResponse>(&url)
        .await
        .unwrap_err();
    assert!(
        format!("{err:?}").contains("503"),
        "expected 503 in final error, got: {err:?}"
    );
}

#[tokio::test]
async fn get_bytes_limited_rejects_oversized_responses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/large.bin"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; 9]))
        .expect(1)
        .mount(&server)
        .await;

    let url = format!("{}/large.bin", server.uri());
    let error = client_without_timeout()
        .get_bytes_limited(&url, 8)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("8-byte limit"));
}

#[tokio::test(start_paused = true)]
async fn download_file_retries_5xx_then_succeeds() {
    let server = MockServer::start().await;
    let attempts = Arc::new(AtomicUsize::new(0));

    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(move |_: &wiremock::Request| {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                ResponseTemplate::new(502)
            } else {
                ResponseTemplate::new(200).set_body_bytes(b"hello, retried".to_vec())
            }
        })
        .expect(2)
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let dest = tmp.path().join("downloaded.bin");
    let url = format!("{}/file.bin", server.uri());

    download_file(&client_without_timeout(), &url, &dest, |_, _| {})
        .await
        .unwrap();

    let content = std::fs::read(&dest).unwrap();
    assert_eq!(content, b"hello, retried");
}

#[tokio::test]
async fn download_file_fails_fast_on_4xx() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/file.bin"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let dest: PathBuf = tmp.path().join("never-written.bin");
    let url = format!("{}/file.bin", server.uri());

    let err = download_file(&HttpClient::new(), &url, &dest, |_, _| {})
        .await
        .unwrap_err();
    assert!(
        format!("{err:?}").contains("404"),
        "expected 404 in error, got: {err:?}"
    );
}

#[tokio::test]
async fn cancelled_downloads_preserve_the_destination_and_remove_all_staging_files() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    for verified in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(connection.read(&mut request).await.unwrap() > 0);
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nx")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("content.jar");
        std::fs::write(&destination, b"original").unwrap();
        let directory = temp.path().to_owned();
        let target = destination.clone();
        let download = tokio::spawn(async move {
            let client = client_without_timeout();
            let url = format!("http://{address}/content.jar");
            if verified {
                let version = serde_json::from_value(json!({
                    "id": "version", "name": "Version", "version_number": "1",
                    "game_versions": [], "loaders": [],
                    "files": [{"url": url, "filename": "content.jar", "size": 100, "primary": true}]
                }))
                .unwrap();
                rmcl::net::modrinth::download_version_file_for_update(
                    &client, &version, &directory, &target,
                )
                .await
                .map(|_| ())
            } else {
                download_file(&client, &url, &target, |_, _| {}).await
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if std::fs::read_dir(temp.path()).unwrap().any(|entry| {
                    let entry = entry.unwrap();
                    entry.path() != destination && entry.metadata().unwrap().len() == 1
                }) {
                    break;
                }
                assert!(
                    !download.is_finished(),
                    "download finished before cancellation"
                );
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        download.abort();
        assert!(download.await.unwrap_err().is_cancelled());
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        assert_eq!(std::fs::read(&destination).unwrap(), b"original");
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            1,
            "verified={verified}"
        );
    }
}

#[tokio::test]
async fn flowing_downloads_can_take_longer_than_thirty_seconds() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut connection, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(connection.read(&mut request).await.unwrap() > 0);
        connection
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nx")
            .await
            .unwrap();
        for _ in 0..7 {
            tokio::time::sleep(std::time::Duration::from_millis(4500)).await;
            if connection.write_all(b"x").await.is_err() {
                break;
            }
        }
    });
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("slow.bin");
    let result = download_file(
        &HttpClient::new(),
        &format!("http://{address}/slow"),
        &path,
        |_, _| {},
    )
    .await;
    server.await.unwrap();
    result.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"xxxxxxxx");
}
