use crate::protocol::{self, Message};
use crate::server::socket_path;
use anyhow::Context;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size as terminal_size};
use std::fs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

pub async fn attach(session: String) -> anyhow::Result<()> {
    let sock = socket_path(&session)?;

    if !sock.exists() {
        anyhow::bail!(
            "No session named '{}' found at {:?}. Use `desktop-tui list` to see active sessions.",
            session,
            sock
        );
    }

    let stream = UnixStream::connect(&sock)
        .await
        .context("Failed to connect to session socket")?;

    eprintln!("[attach] Connected to session '{}'.", session);

    // Put the local terminal into raw mode so every keystroke is forwarded.
    enable_raw_mode().context("Failed to enable raw mode")?;

    let (mut reader, mut writer) = stream.into_split();

    // Send initial resize before entering the event loop.
    if let Ok((cols, rows)) = terminal_size() {
        let msg = Message::Resize { cols, rows };
        let encoded = protocol::encode(&msg)?;
        writer.write_all(&encoded).await?;
    }

    // Task: read from server, write to stdout.
    let stdout_task = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        loop {
            match protocol::decode(&mut reader).await {
                Ok(Message::Data(bytes)) => {
                    if stdout.write_all(&bytes).await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
                Ok(Message::Detach) | Err(_) => break,
                _ => {}
            }
        }
    });

    // Task: read from stdin, send to server.
    let stdin_task = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buf = vec![0u8; 1024];
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let data = buf[..n].to_vec();
                    let msg = Message::Data(data);
                    match protocol::encode(&msg) {
                        Ok(encoded) => {
                            if writer.write_all(&encoded).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }
        }
    });

    // Wait for either task to finish (client disconnect or server gone).
    tokio::select! {
        _ = stdout_task => {}
        _ = stdin_task => {}
    }

    // Restore terminal mode before returning.
    let _ = disable_raw_mode();
    eprintln!("\r\n[attach] Detached from session '{}'.", session);

    Ok(())
}

pub fn list_sessions() -> anyhow::Result<()> {
    let home = std::env::var("HOME").context("HOME env var not set")?;
    let dir = std::path::PathBuf::from(home).join(".local/share/desktop-tui");

    if !dir.exists() {
        println!("No sessions found (session directory does not exist).");
        return Ok(());
    }

    let entries = fs::read_dir(&dir).context("Failed to read session directory")?;

    let mut found = false;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("sock") {
            continue;
        }

        let session_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("<unknown>");

        // Check if socket is actually alive by attempting a connection.
        let alive = std::os::unix::net::UnixStream::connect(&path).is_ok();

        if alive {
            println!("  {} (active)", session_name);
            found = true;
        } else {
            println!("  {} (stale)", session_name);
            found = true;
        }
    }

    if !found {
        println!("No sessions found.");
    }

    Ok(())
}
