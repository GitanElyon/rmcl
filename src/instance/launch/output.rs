// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};

use super::parser::LogStream;

pub(super) type LogWriter = Arc<Mutex<Option<std::fs::File>>>;

pub(super) async fn capture(
    reader: impl tokio::io::AsyncRead + Unpin,
    stream: LogStream,
    sender: tokio::sync::mpsc::Sender<(LogStream, String)>,
    writer: LogWriter,
) {
    use std::io::Write;
    let mut reader = tokio::io::BufReader::new(reader);
    loop {
        let mut bytes = Vec::new();
        match (&mut reader)
            .take(64 * 1024)
            .read_until(b'\n', &mut bytes)
            .await
        {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                tracing::warn!("Minecraft output read failed: {error}");
                break;
            }
        }
        if let Ok(mut file) = writer.lock()
            && let Some(file) = file.as_mut()
            && let Err(error) = file.write_all(&bytes)
        {
            tracing::warn!("Minecraft log write failed: {error}");
        }
        let line = String::from_utf8_lossy(&bytes)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        if sender.send((stream, line)).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_utf8_and_long_lines_keep_following_output() {
        let mut bytes = vec![b'x'; 128 * 1024];
        bytes.extend_from_slice(b"\ninvalid \xff\nlast\n");
        let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
        capture(
            bytes.as_slice(),
            LogStream::Stdout,
            sender,
            Default::default(),
        )
        .await;
        let mut lines = Vec::new();
        while let Some((_, line)) = receiver.recv().await {
            lines.push(line);
        }
        assert!(lines.iter().all(|line| line.len() <= 64 * 1024));
        assert!(lines.iter().any(|line| line == "invalid �"));
        assert_eq!(lines.last().unwrap(), "last");
    }
}
