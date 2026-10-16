//! The single interactive shell session behind the terminal view.
//!
//! Output streams to the frontend over a Tauri `Channel` as UTF-8 text (split
//! multi-byte sequences are carried over between reads). When the shell exits
//! we emit `pty://exit` after the last output chunk has been delivered.

use crate::{config, shell};
use portable_pty::{native_pty_system, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde::Serialize;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Emitter, State};

struct Session {
    generation: u64,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    killer: Box<dyn ChildKiller + Send + Sync>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.killer.kill();
    }
}

#[derive(Default)]
pub struct PtyState {
    session: Arc<Mutex<Option<Session>>>,
    generation: AtomicU64,
}

impl PtyState {
    pub fn shutdown(&self) {
        self.session.lock().unwrap().take();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnInfo {
    shell: String,
    args: Vec<String>,
    cwd: String,
    windows_build: Option<u32>,
}

#[derive(Clone, Serialize)]
struct PtyExit {
    code: Option<u32>,
}

fn windows_build() -> Option<u32> {
    if !cfg!(windows) {
        return None;
    }
    match os_info::get().version() {
        os_info::Version::Semantic(_, _, build) => u32::try_from(*build).ok(),
        _ => None,
    }
}

fn start_dir(cfg: &config::Config) -> PathBuf {
    cfg.start_dir
        .as_deref()
        .map(config::expand_home)
        .filter(|p| p.is_dir())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Starts (or restarts) the shell. Any existing session is killed first.
#[tauri::command]
pub async fn pty_spawn(
    app: AppHandle,
    state: State<'_, PtyState>,
    cols: u16,
    rows: u16,
    on_data: Channel<String>,
) -> Result<SpawnInfo, String> {
    state.shutdown();

    let (cfg, warning) = config::load();
    if let Some(w) = warning {
        let _ = on_data.send(format!("\x1b[33m{w}\x1b[0m\r\n"));
    }
    let spec = shell::resolve(&cfg)?;
    let cwd = start_dir(&cfg);

    let pair = native_pty_system()
        .openpty(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 })
        .map_err(|e| format!("Could not open a pseudo-terminal: {e}"))?;

    let mut cmd = CommandBuilder::new(&spec.program);
    cmd.args(&spec.args);
    cmd.cwd(&cwd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("TERM_PROGRAM", "branchdeck");
    cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));

    let mut child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("Could not start {}: {e}", spec.program.display()))?;
    // The slave end belongs to the child now; holding it would keep the pty open after exit.
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let killer = child.clone_killer();

    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    *state.session.lock().unwrap() = Some(Session { generation, master: pair.master, writer, killer });

    let reader_thread = std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        let mut pending = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = decode_utf8_stream(&mut pending, &buf[..n]);
                    if !text.is_empty() && on_data.send(text).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let sessions = state.session.clone();
    std::thread::spawn(move || {
        let status = child.wait().ok();
        // Let ConPTY flush its last frame, then close the master so the reader sees EOF.
        std::thread::sleep(Duration::from_millis(100));
        let current = {
            let mut guard = sessions.lock().unwrap();
            if guard.as_ref().map(|s| s.generation) == Some(generation) {
                guard.take();
                true
            } else {
                false
            }
        };
        let _ = reader_thread.join();
        // A replaced session must not tell the UI that "the" shell exited.
        if current {
            let _ = app.emit("pty://exit", PtyExit { code: status.map(|s| s.exit_code()) });
        }
    });

    Ok(SpawnInfo {
        shell: spec.program.display().to_string(),
        args: spec.args,
        cwd: cwd.display().to_string(),
        windows_build: windows_build(),
    })
}

#[tauri::command]
pub async fn pty_write(state: State<'_, PtyState>, data: String) -> Result<(), String> {
    let mut guard = state.session.lock().unwrap();
    let session = guard.as_mut().ok_or("No shell is running")?;
    session.writer.write_all(data.as_bytes()).map_err(|e| e.to_string())?;
    session.writer.flush().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pty_resize(state: State<'_, PtyState>, cols: u16, rows: u16) -> Result<(), String> {
    let guard = state.session.lock().unwrap();
    let Some(session) = guard.as_ref() else { return Ok(()) };
    session
        .master
        .resize(PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 })
        .map_err(|e| e.to_string())
}

/// Appends `chunk` to `pending` and returns everything that decodes cleanly.
/// An incomplete multi-byte sequence at the end stays in `pending` for the next
/// read; truly invalid bytes become U+FFFD.
fn decode_utf8_stream(pending: &mut Vec<u8>, chunk: &[u8]) -> String {
    pending.extend_from_slice(chunk);
    let mut out = String::with_capacity(pending.len());
    let mut start = 0;
    while start < pending.len() {
        match std::str::from_utf8(&pending[start..]) {
            Ok(s) => {
                out.push_str(s);
                start = pending.len();
            }
            Err(e) => {
                let valid = e.valid_up_to();
                // from_utf8 just told us this prefix is valid.
                out.push_str(std::str::from_utf8(&pending[start..start + valid]).unwrap());
                match e.error_len() {
                    Some(bad) => {
                        out.push('\u{FFFD}');
                        start += valid + bad;
                    }
                    None => {
                        start += valid;
                        break;
                    }
                }
            }
        }
    }
    pending.drain(..start);
    out
}

#[cfg(test)]
mod tests {
    use super::decode_utf8_stream;

    #[test]
    fn passes_ascii_through() {
        let mut p = Vec::new();
        assert_eq!(decode_utf8_stream(&mut p, b"hello"), "hello");
        assert!(p.is_empty());
    }

    #[test]
    fn carries_split_multibyte_sequence() {
        let bytes = "a→b".as_bytes(); // → is 3 bytes
        let mut p = Vec::new();
        assert_eq!(decode_utf8_stream(&mut p, &bytes[..2]), "a");
        assert_eq!(p.len(), 1);
        assert_eq!(decode_utf8_stream(&mut p, &bytes[2..3]), "");
        assert_eq!(decode_utf8_stream(&mut p, &bytes[3..]), "→b");
        assert!(p.is_empty());
    }

    /// End-to-end through a real pseudo-terminal with the auto-detected shell.
    #[test]
    fn default_shell_runs_in_pty() {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};
        use std::io::{Read, Write};

        let spec = crate::shell::resolve(&crate::config::Config::default()).unwrap();
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.env("TERM", "xterm-256color");
        let mut child = pair.slave.spawn_command(cmd).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let mut writer = pair.master.take_writer().unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            let mut buf = [0u8; 4096];
            let mut sent = false;
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let chunk = &buf[..n];
                out.extend_from_slice(chunk);
                // ConPTY blocks on a cursor-position query until the terminal
                // answers; xterm.js does this in the app, we play terminal here.
                if chunk.windows(4).any(|w| w == b"\x1b[6n") {
                    writer.write_all(b"\x1b[1;1R").unwrap();
                }
                if !sent {
                    // Arithmetic so the marker only appears in output, not the echoed input.
                    writer.write_all(b"echo bd-$((40+2))-ok; exit\r").unwrap();
                    sent = true;
                }
                writer.flush().unwrap();
                if String::from_utf8_lossy(&out).contains("bd-42-ok") {
                    let _ = tx.send(true);
                    return;
                }
            }
            eprintln!("PTY OUTPUT: {:?}", String::from_utf8_lossy(&out));
            let _ = tx.send(false);
        });
        let found = rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or(false);
        let _ = child.kill();
        assert!(found, "shell output never contained the marker");
    }

    #[test]
    fn replaces_invalid_bytes() {
        let mut p = Vec::new();
        assert_eq!(decode_utf8_stream(&mut p, b"a\xffb"), "a\u{FFFD}b");
        assert!(p.is_empty());
    }
}
