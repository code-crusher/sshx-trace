//! Defines tasks that control the behavior of a single shell in the client.

use anyhow::Result;
use encoding_rs::{CoderResult, UTF_8};
use sshx_core::proto::{client_update::ClientMessage, TerminalData};
use sshx_core::Sid;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

use crate::encrypt::Encrypt;
use crate::terminal::Terminal;

const CONTENT_CHUNK_SIZE: usize = 1 << 16; // Send at most this many bytes at a time.
const CONTENT_ROLLING_BYTES: usize = 8 << 20; // Store at least this much content.
const CONTENT_PRUNE_BYTES: usize = 12 << 20; // Prune when we exceed this length.

/// Escape sequences that ask the terminal to report the cursor position
/// (Device Status Report, DSR), in plain and DEC-specific forms.
///
/// Programs such as nushell write one of these and then block until the
/// terminal replies. The real terminal lives in the browser, which does not
/// answer (see the sshx-xterm patches), so the shell subprocess would hang
/// forever. We answer on its behalf here, in the single client process that
/// owns the PTY, so each query gets exactly one reply no matter how many users
/// are watching. See <https://github.com/ekzhang/sshx/issues/60>.
const CURSOR_POSITION_QUERY: &[u8] = b"\x1b[6n";
const CURSOR_POSITION_QUERY_PRIVATE: &[u8] = b"\x1b[?6n";

/// Bytes to remember between reads, so a query split across two reads is still
/// recognized. This is the length of the longest query minus one.
const QUERY_TAIL_LEN: usize = CURSOR_POSITION_QUERY_PRIVATE.len() - 1;

/// Detects cursor position queries in terminal output and answers them.
///
/// Replies report the cursor at the start of the last row. Line editors such as
/// nushell use that position to decide where to redraw their prompt: claiming
/// the first row makes them clear the screen on every repaint, while the last
/// row preserves the output above and scrolls naturally.
#[derive(Debug, Default)]
struct CursorPositionFilter {
    /// Trailing bytes of the previous chunk, which may begin a query.
    tail: Vec<u8>,
}

impl CursorPositionFilter {
    /// Scans a chunk of terminal output, returning a reply to forward to the
    /// shell if it contains any cursor position queries.
    fn reply(&mut self, data: &[u8], rows: u16) -> Option<Vec<u8>> {
        let tail_len = self.tail.len();
        let mut input = std::mem::take(&mut self.tail);
        input.extend_from_slice(data);

        let mut reply = Vec::new();
        let mut i = 0;
        while i < input.len() {
            let rest = &input[i..];
            let (len, private) = if rest.starts_with(CURSOR_POSITION_QUERY_PRIVATE) {
                (CURSOR_POSITION_QUERY_PRIVATE.len(), "?")
            } else if rest.starts_with(CURSOR_POSITION_QUERY) {
                (CURSOR_POSITION_QUERY.len(), "")
            } else {
                i += 1;
                continue;
            };
            // A query entirely inside the tail was already answered last time.
            if i + len > tail_len {
                let row = rows.max(1); // cursor positions are 1-based
                reply.extend_from_slice(format!("\x1b[{private}{row};1R").as_bytes());
            }
            i += len;
        }

        let keep = QUERY_TAIL_LEN.min(input.len());
        self.tail = input[input.len() - keep..].to_vec();
        (!reply.is_empty()).then_some(reply)
    }
}

/// Variants of terminal behavior that are used by the controller.
#[derive(Debug, Clone)]
pub enum Runner {
    /// Spawns the specified shell as a subprocess, forwarding PTYs.
    Shell(String),

    /// Mock runner that only echos its input, useful for testing.
    Echo,
}

/// Internal message routed to shell runners.
pub enum ShellData {
    /// Sequence of input bytes from the server.
    Data(Vec<u8>),
    /// Information about the server's current sequence number.
    Sync(u64),
    /// Resize the shell to a different number of rows and columns.
    Size(u32, u32),
}

impl Runner {
    /// Asynchronous task to run a single shell with process I/O.
    pub async fn run(
        &self,
        id: Sid,
        encrypt: Encrypt,
        shell_rx: mpsc::Receiver<ShellData>,
        output_tx: mpsc::Sender<ClientMessage>,
    ) -> Result<()> {
        match self {
            Self::Shell(shell) => shell_task(id, encrypt, shell, shell_rx, output_tx).await,
            Self::Echo => echo_task(id, encrypt, shell_rx, output_tx).await,
        }
    }
}

/// Asynchronous task handling a single shell within the session.
async fn shell_task(
    id: Sid,
    encrypt: Encrypt,
    shell: &str,
    mut shell_rx: mpsc::Receiver<ShellData>,
    output_tx: mpsc::Sender<ClientMessage>,
) -> Result<()> {
    let mut term = Terminal::new(shell).await?;
    let mut rows = 24; // terminal height, used to answer cursor queries
    term.set_winsize(rows, 80)?;

    let mut content = String::new(); // content from the terminal
    let mut content_offset = 0; // bytes before the first character of `content`
    let mut decoder = UTF_8.new_decoder(); // UTF-8 streaming decoder
    let mut cursor_filter = CursorPositionFilter::default(); // answers DSR queries
    let mut seq = 0; // our log of the server's sequence number
    let mut seq_outdated = 0; // number of times seq has been outdated
    let mut buf = [0u8; 4096]; // buffer for reading
    let mut finished = false; // set when this is done

    while !finished {
        tokio::select! {
            result = term.read(&mut buf) => {
                let n = result?;
                if n == 0 {
                    finished = true;
                } else {
                    if let Some(reply) = cursor_filter.reply(&buf[..n], rows) {
                        term.write_all(&reply).await?;
                    }
                    content.reserve(decoder.max_utf8_buffer_length(n).unwrap());
                    let (result, _, _) = decoder.decode_to_string(&buf[..n], &mut content, false);
                    debug_assert!(result == CoderResult::InputEmpty);
                }
            }
            item = shell_rx.recv() => {
                match item {
                    Some(ShellData::Data(data)) => {
                        term.write_all(&data).await?;
                    }
                    Some(ShellData::Sync(seq2)) => {
                        if seq2 < seq as u64 {
                            seq_outdated += 1;
                            if seq_outdated >= 3 {
                                seq = seq2 as usize;
                            }
                        }
                    }
                    Some(ShellData::Size(new_rows, cols)) => {
                        term.set_winsize(new_rows as u16, cols as u16)?;
                        rows = new_rows as u16;
                    }
                    None => finished = true, // Server closed this shell.
                }
            }
        }

        if finished {
            content.reserve(decoder.max_utf8_buffer_length(0).unwrap());
            let (result, _, _) = decoder.decode_to_string(&[], &mut content, true);
            debug_assert!(result == CoderResult::InputEmpty);
        }

        // Send data if the server has fallen behind.
        if content_offset + content.len() > seq {
            let start = prev_char_boundary(&content, seq - content_offset);
            let end = prev_char_boundary(&content, (start + CONTENT_CHUNK_SIZE).min(content.len()));
            let data = encrypt.segment(
                0x100000000 | id.0 as u64, // stream number
                (content_offset + start) as u64,
                &content.as_bytes()[start..end],
            );
            let data = TerminalData {
                id: id.0,
                data: data.into(),
                seq: (content_offset + start) as u64,
            };
            output_tx.send(ClientMessage::Data(data)).await?;
            seq = content_offset + end;
            seq_outdated = 0;
        }

        if content.len() > CONTENT_PRUNE_BYTES && seq - CONTENT_ROLLING_BYTES > content_offset {
            let pruned = (seq - CONTENT_ROLLING_BYTES) - content_offset;
            let pruned = prev_char_boundary(&content, pruned);
            content_offset += pruned;
            content.drain(..pruned);
        }
    }
    Ok(())
}

/// Find the last char boundary before an index in O(1) time.
fn prev_char_boundary(s: &str, i: usize) -> usize {
    (0..=i)
        .rev()
        .find(|&j| s.is_char_boundary(j))
        .expect("no previous char boundary")
}

async fn echo_task(
    id: Sid,
    encrypt: Encrypt,
    mut shell_rx: mpsc::Receiver<ShellData>,
    output_tx: mpsc::Sender<ClientMessage>,
) -> Result<()> {
    let mut seq = 0;
    while let Some(item) = shell_rx.recv().await {
        match item {
            ShellData::Data(data) => {
                let msg = String::from_utf8_lossy(&data);
                let term_data = TerminalData {
                    id: id.0,
                    data: encrypt
                        .segment(0x100000000 | id.0 as u64, seq, msg.as_bytes())
                        .into(),
                    seq,
                };
                output_tx.send(ClientMessage::Data(term_data)).await?;
                seq += msg.len() as u64;
            }
            ShellData::Sync(_) => (),
            ShellData::Size(_, _) => (),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CursorPositionFilter, Runner};
    use crate::encrypt::Encrypt;
    use sshx_core::proto::client_update::ClientMessage;
    use sshx_core::Sid;
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// Runs a shell that blocks until the terminal reports the cursor position,
    /// then prints the reply. This is what nushell does on startup (#60).
    #[cfg(unix)]
    #[tokio::test]
    async fn answers_cursor_query_from_shell() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("sshx-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("cursor-query.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n\
             stty raw -echo\n\
             printf '\\033[6n'\n\
             reply=''\n\
             while :; do\n\
             \tchar=$(dd bs=1 count=1 2>/dev/null)\n\
             \treply=\"$reply$char\"\n\
             \tcase \"$char\" in R) break;; esac\n\
             done\n\
             stty sane\n\
             printf 'reply=[%s]\\r\\n' \"$reply\"\n\
             exec sleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let key = "test";
        let (shell_tx, shell_rx) = mpsc::channel(16);
        let (output_tx, mut output_rx) = mpsc::channel(16);
        let runner = Runner::Shell(script.to_string_lossy().into_owned());
        let task = tokio::spawn(async move {
            runner
                .run(Sid(1), Encrypt::new(key), shell_rx, output_tx)
                .await
        });

        // Decrypt the shell's output until it reports the reply it received.
        let encrypt = Encrypt::new(key);
        let mut output = String::new();
        while !output.contains("reply=[") || !output.contains("]") {
            let message = tokio::time::timeout(Duration::from_secs(10), output_rx.recv())
                .await
                .expect("shell hung waiting for a cursor position reply")
                .expect("shell task exited unexpectedly");
            if let ClientMessage::Data(data) = message {
                let decrypted = encrypt.segment(0x100000000 | 1, data.seq, &data.data);
                output.push_str(&String::from_utf8_lossy(&decrypted));
            }
        }

        let reply = output
            .split("reply=[")
            .nth(1)
            .and_then(|rest| rest.split(']').next())
            .unwrap();
        assert_eq!(reply, "\x1b[24;1R");

        // Closing the channel terminates the shell task.
        drop(shell_tx);
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("shell task did not exit")
            .unwrap()
            .unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_query() {
        let mut filter = CursorPositionFilter::default();
        assert_eq!(filter.reply(b"hello world", 24), None);
        assert_eq!(filter.reply(b"\x1b[0m\r\n", 24), None);
    }

    #[test]
    fn answers_query() {
        let mut filter = CursorPositionFilter::default();
        assert_eq!(
            filter.reply(b"prompt\x1b[6n", 24),
            Some(b"\x1b[24;1R".to_vec())
        );
    }

    #[test]
    fn answers_private_query() {
        let mut filter = CursorPositionFilter::default();
        assert_eq!(filter.reply(b"\x1b[?6n", 10), Some(b"\x1b[?10;1R".to_vec()));
    }

    #[test]
    fn answers_each_query_once() {
        let mut filter = CursorPositionFilter::default();
        assert_eq!(
            filter.reply(b"\x1b[6n\x1b[6n\x1b[?6n", 24),
            Some(b"\x1b[24;1R\x1b[24;1R\x1b[?24;1R".to_vec())
        );
        // A query already answered must not be answered again later.
        assert_eq!(filter.reply(b"plain output", 24), None);
    }

    #[test]
    fn answers_split_query_once() {
        let mut filter = CursorPositionFilter::default();
        assert_eq!(filter.reply(b"\x1b[", 24), None);
        assert_eq!(filter.reply(b"6n", 24), Some(b"\x1b[24;1R".to_vec()));
        assert_eq!(filter.reply(b"", 24), None);
    }

    #[test]
    fn answers_split_private_query_once() {
        let mut filter = CursorPositionFilter::default();
        assert_eq!(filter.reply(b"\x1b[?", 24), None);
        assert_eq!(filter.reply(b"6", 24), None);
        assert_eq!(filter.reply(b"n", 24), Some(b"\x1b[?24;1R".to_vec()));
        assert_eq!(filter.reply(b"n", 24), None);
    }

    #[test]
    fn ignores_unrelated_sequences() {
        let mut filter = CursorPositionFilter::default();
        assert_eq!(filter.reply(b"\x1b[?25l\x1b[1;1H\x1b[J", 24), None);
    }
}
