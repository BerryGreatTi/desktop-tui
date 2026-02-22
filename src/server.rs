use crate::protocol::{self, Message};
use anyhow::{anyhow, Context};
use nix::pty::{openpty, Winsize};
use nix::sys::signal::{kill, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;
use std::fs;
use std::os::fd::{FromRawFd, IntoRawFd};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, Mutex};

/// Default terminal size used when spawning the child PTY process.
const DEFAULT_COLS: u16 = 220;
const DEFAULT_ROWS: u16 = 50;

/// Return the session directory, creating it if needed.
fn session_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME env var not set")?;
    let dir = PathBuf::from(home).join(".local/share/desktop-tui");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Return the socket path for the given session name.
pub fn socket_path(session: &str) -> anyhow::Result<PathBuf> {
    Ok(session_dir()?.join(format!("{}.sock", session)))
}

pub async fn serve(shortcut_dir: PathBuf, session: String) -> anyhow::Result<()> {
    let sock_path = socket_path(&session)?;

    // Remove stale socket if it exists.
    if sock_path.exists() {
        fs::remove_file(&sock_path)?;
    }

    // Open a PTY pair.
    let winsize = Winsize {
        ws_col: DEFAULT_COLS,
        ws_row: DEFAULT_ROWS,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let pty = openpty(Some(&winsize), None).context("openpty failed")?;

    // Raw FDs so we can hand them to the child and keep the master ourselves.
    let master_fd = pty.master.into_raw_fd();
    let slave_fd = pty.slave.into_raw_fd();

    // Build the child command. We re-exec the current binary with `run`.
    let exe = std::env::current_exe().context("cannot determine current executable path")?;
    let shortcut_dir_str = shortcut_dir
        .to_str()
        .ok_or_else(|| anyhow!("shortcut_dir is not valid UTF-8"))?
        .to_owned();

    // Spawn child with PTY slave as its stdio.
    // pre_exec is used (not exec() shell invocation) to avoid command injection:
    // we duplicate the slave FD onto stdio descriptors inside the child process,
    // then the OS exec replaces the process image with the exact binary path.
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("run").arg(&shortcut_dir_str);

    // Safety: pre_exec runs in the forked child before exec.
    // We redirect stdin/stdout/stderr to the PTY slave and close the master.
    unsafe {
        cmd.pre_exec(move || {
            // Redirect stdio to slave PTY.
            libc::dup2(slave_fd, libc::STDIN_FILENO);
            libc::dup2(slave_fd, libc::STDOUT_FILENO);
            libc::dup2(slave_fd, libc::STDERR_FILENO);

            // Close the extra slave FD (was duplicated above).
            if slave_fd > 2 {
                libc::close(slave_fd);
            }

            // Create a new session so the child owns the terminal.
            libc::setsid();

            Ok(())
        });
    }

    let child = cmd.spawn().context("failed to spawn desktop-tui run child")?;
    let child_pid = Pid::from_raw(child.id() as i32);

    // Close slave FD in the parent now that the child has inherited it.
    unsafe { libc::close(slave_fd) };

    // Wrap the master FD for async reading and writing.
    // Duplicate so we can have independent read and write handles.
    let master_file_read = unsafe { std::fs::File::from_raw_fd(master_fd) };
    let master_fd_write = unsafe { libc::dup(master_fd) };
    let master_file_write = unsafe { std::fs::File::from_raw_fd(master_fd_write) };

    let master_read = Arc::new(Mutex::new(tokio::fs::File::from_std(master_file_read)));
    let master_write = Arc::new(Mutex::new(tokio::fs::File::from_std(master_file_write)));

    // Broadcast channel: PTY output -> all connected clients.
    let (pty_tx, _pty_rx) = broadcast::channel::<Vec<u8>>(256);
    let pty_tx = Arc::new(pty_tx);

    // Spawn task: continuously read from PTY master and broadcast.
    {
        let pty_tx = Arc::clone(&pty_tx);
        let master_read = Arc::clone(&master_read);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            loop {
                let n = {
                    let mut guard = master_read.lock().await;
                    match guard.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    }
                };
                let data = buf[..n].to_vec();
                // Ignore send errors (no receivers connected yet is fine).
                let _ = pty_tx.send(data);
            }
        });
    }

    // Unix socket listener.
    let listener = UnixListener::bind(&sock_path).context("failed to bind Unix socket")?;
    eprintln!("[serve] Session '{}' listening on {:?}", session, sock_path);

    // Accept clients in a loop.
    loop {
        // Check if child has exited.
        match waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _)) => {
                eprintln!("[serve] Child process exited, shutting down.");
                break;
            }
            _ => {}
        }

        // Accept a new connection with a short timeout so we can re-check child status.
        let stream = tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        eprintln!("[serve] Accept error: {}", e);
                        continue;
                    }
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
                continue;
            }
        };

        eprintln!("[serve] Client connected.");
        let pty_rx = pty_tx.subscribe();
        let master_write = Arc::clone(&master_write);

        tokio::spawn(handle_client(stream, pty_rx, master_write, child_pid, master_fd));
    }

    // Clean up socket file.
    let _ = fs::remove_file(&sock_path);
    Ok(())
}

async fn handle_client(
    stream: UnixStream,
    mut pty_rx: broadcast::Receiver<Vec<u8>>,
    master_write: Arc<Mutex<tokio::fs::File>>,
    child_pid: Pid,
    master_fd: i32,
) {
    let (mut reader, mut writer) = stream.into_split();

    loop {
        tokio::select! {
            // Data from PTY -> send to client.
            result = pty_rx.recv() => {
                match result {
                    Ok(data) => {
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
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        // Skip lagged messages and continue.
                        continue;
                    }
                    Err(_) => break,
                }
            }

            // Message from client.
            result = protocol::decode(&mut reader) => {
                match result {
                    Ok(Message::Data(bytes)) => {
                        let mut guard = master_write.lock().await;
                        if guard.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Ok(Message::Resize { cols, rows }) => {
                        let winsize = Winsize {
                            ws_col: cols,
                            ws_row: rows,
                            ws_xpixel: 0,
                            ws_ypixel: 0,
                        };
                        // Set PTY window size.
                        unsafe {
                            libc::ioctl(
                                master_fd,
                                libc::TIOCSWINSZ,
                                &winsize as *const Winsize,
                            );
                        }
                        // Notify the child of the resize.
                        let _ = kill(child_pid, Signal::SIGWINCH);
                    }
                    Ok(Message::Detach) => {
                        eprintln!("[serve] Client detached.");
                        break;
                    }
                    Ok(Message::Shutdown) => {
                        eprintln!("[serve] Client requested shutdown.");
                        let _ = kill(child_pid, Signal::SIGTERM);
                        break;
                    }
                    Err(_) => break,
                }
            }
        }
    }

    eprintln!("[serve] Client disconnected.");
}
