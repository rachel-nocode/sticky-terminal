pub(crate) mod clipboard;

use eframe::egui;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use vt100::Parser;

use crate::terminal::clipboard::{read_clipboard, save_clipboard_image};

const TERMINAL_SCROLLBACK: usize = 5_000;

#[derive(Clone, Copy)]
pub(crate) struct TerminalPoint {
    pub(crate) row: u16,
    pub(crate) col: u16,
}

// ── A single terminal session (pane) ──

pub(crate) struct TerminalPane {
    pub(crate) cwd: PathBuf,
    pub(crate) parser: Parser,
    pub(crate) rx: Option<Receiver<Vec<u8>>>,
    pub(crate) writer: Option<Box<dyn Write + Send>>,
    pub(crate) master: Option<Box<dyn MasterPty + Send>>,
    pub(crate) child: Option<Box<dyn Child + Send + Sync>>,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) status: String,
    pub(crate) has_focus: bool,
    pub(crate) selection: Option<(TerminalPoint, TerminalPoint)>,
    pub(crate) paste_chip: Option<String>,
    pub(crate) pending_logs: Vec<String>,
    pub(crate) mirror_tx: Option<std::sync::mpsc::SyncSender<Vec<u8>>>,
    pub(crate) detected_port: Option<u16>,
    // ── Shell integration (OSC 133 / OSC 7) ──
    /// True between a command's preexec (`133;C`) and its completion (`133;D`).
    pub(crate) command_running: bool,
    /// Exit code of the last finished command, parsed from `133;D;<code>`.
    pub(crate) last_exit: Option<i32>,
    /// Live cwd reported by the shell via OSC 7.
    pub(crate) live_cwd: Option<PathBuf>,
    /// Carry buffer for OSC sequences split across PTY reads.
    osc_buf: Vec<u8>,
}

impl TerminalPane {
    pub(crate) fn new(cwd: PathBuf) -> Self {
        let rows = 28;
        let cols = 120;

        Self {
            cwd,
            parser: Parser::new(rows, cols, TERMINAL_SCROLLBACK),
            rx: None,
            writer: None,
            master: None,
            child: None,
            rows,
            cols,
            status: "Starting shell...".to_owned(),
            has_focus: false,
            selection: None,
            paste_chip: None,
            pending_logs: Vec::new(),
            mirror_tx: None,
            detected_port: None,
            command_running: false,
            last_exit: None,
            live_cwd: None,
            osc_buf: Vec::new(),
        }
    }

    /// Scan PTY output for OSC 133 (command boundaries) and OSC 7 (cwd).
    /// vt100 ignores these sequences, so we parse them ourselves.
    fn scan_osc(&mut self, bytes: &[u8]) {
        self.osc_buf.extend_from_slice(bytes);
        if self.osc_buf.len() > 8192 {
            let drop = self.osc_buf.len() - 1024;
            self.osc_buf.drain(..drop);
        }
        loop {
            let Some(start) = self
                .osc_buf
                .windows(2)
                .position(|w| w == [0x1b, b']'])
            else {
                // No OSC introducer — keep only a trailing lone ESC.
                let keep_esc = self.osc_buf.last() == Some(&0x1b);
                self.osc_buf.clear();
                if keep_esc {
                    self.osc_buf.push(0x1b);
                }
                break;
            };
            let body = &self.osc_buf[start + 2..];
            let term = body
                .iter()
                .position(|&b| b == 0x07)
                .map(|p| (p, 1usize))
                .or_else(|| {
                    body.windows(2)
                        .position(|w| w == [0x1b, b'\\'])
                        .map(|p| (p, 2usize))
                });
            let Some((rel_end, term_len)) = term else {
                // Incomplete sequence — wait for the rest.
                self.osc_buf.drain(..start);
                break;
            };
            let payload = body[..rel_end].to_vec();
            self.handle_osc(&payload);
            let consumed = start + 2 + rel_end + term_len;
            self.osc_buf.drain(..consumed);
        }
    }

    fn handle_osc(&mut self, payload: &[u8]) {
        let Ok(s) = std::str::from_utf8(payload) else {
            return;
        };
        if let Some(rest) = s.strip_prefix("133;") {
            match rest.as_bytes().first() {
                Some(b'C') => self.command_running = true,
                Some(b'D') => {
                    self.command_running = false;
                    if let Some(code) = rest.strip_prefix("D;") {
                        self.last_exit = code.trim().parse().ok();
                    }
                }
                _ => {}
            }
        } else if let Some(rest) = s.strip_prefix("7;") {
            if let Some(url) = rest.strip_prefix("file://") {
                // Drop the host component; keep the absolute path.
                if let Some(slash) = url.find('/') {
                    self.live_cwd = Some(PathBuf::from(percent_decode(&url[slash..])));
                }
            }
        }
    }

    fn shell_builder(&self) -> CommandBuilder {
        #[cfg(target_os = "windows")]
        {
            let mut command = CommandBuilder::new("cmd.exe");
            command.cwd(self.cwd.as_os_str());
            command
        }

        #[cfg(not(target_os = "windows"))]
        {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
            let mut command = CommandBuilder::new(&shell);
            command.arg("-il");
            command.cwd(self.cwd.as_os_str());
            command.env("TERM", "xterm-256color");
            command.env("COLORTERM", "truecolor");

            // zsh: inject OSC 133 / OSC 7 shell integration via a temp ZDOTDIR.
            if shell.rsplit('/').next() == Some("zsh") {
                if let Some(dir) = ensure_zsh_integration() {
                    let user_zdotdir = std::env::var("ZDOTDIR")
                        .ok()
                        .filter(|v| !v.is_empty())
                        .or_else(|| std::env::var("HOME").ok())
                        .unwrap_or_default();
                    command.env("USER_ZDOTDIR", user_zdotdir);
                    command.env("ZDOTDIR", dir);
                }
            }
            command
        }
    }

    pub(crate) fn ensure_started(&mut self) {
        if self.rx.is_some() {
            return;
        }

        let pty_system = native_pty_system();
        let pair = match pty_system.openpty(PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }) {
            Ok(pair) => pair,
            Err(err) => {
                self.status = format!("Could not open terminal: {err}");
                return;
            }
        };

        let command = self.shell_builder();
        let child = match pair.slave.spawn_command(command) {
            Ok(child) => child,
            Err(err) => {
                self.status = format!("Could not start shell: {err}");
                return;
            }
        };

        let writer = match pair.master.take_writer() {
            Ok(writer) => writer,
            Err(err) => {
                self.status = format!("Could not connect terminal input: {err}");
                return;
            }
        };

        let mut reader = match pair.master.try_clone_reader() {
            Ok(reader) => reader,
            Err(err) => {
                self.status = format!("Could not connect terminal output: {err}");
                return;
            }
        };

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if tx.send(buffer[..count].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let message = format!("\r\n[terminal read error: {err}]\r\n");
                        let _ = tx.send(message.into_bytes());
                        break;
                    }
                }
            }
        });

        self.writer = Some(writer);
        self.master = Some(pair.master);
        self.child = Some(child);
        self.rx = Some(rx);
        self.status = format!("Interactive shell in {}", self.cwd.display());
    }

    pub(crate) fn set_scrollback(&mut self, rows: usize) {
        self.parser.screen_mut().set_scrollback(rows);
    }

    pub(crate) fn scrollback_position(&self) -> usize {
        self.parser.screen().scrollback()
    }

    pub(crate) fn max_scrollback(&mut self) -> usize {
        let current = self.scrollback_position();
        self.parser.screen_mut().set_scrollback(usize::MAX);
        let max = self.parser.screen().scrollback();
        self.parser.screen_mut().set_scrollback(current.min(max));
        max
    }

    pub(crate) fn adjust_scrollback(&mut self, delta_rows: i32) {
        let current = self.parser.screen().scrollback() as i32;
        let next = (current + delta_rows).max(0) as usize;
        self.set_scrollback(next);
    }

    pub(crate) fn drain_output(&mut self) -> bool {
        let mut chunks: Vec<Vec<u8>> = Vec::new();
        if let Some(rx) = &self.rx {
            while let Ok(bytes) = rx.try_recv() {
                chunks.push(bytes);
            }
        }

        let received_output = !chunks.is_empty();
        for bytes in &chunks {
            self.parser.process(bytes);
            if let Some(tx) = &self.mirror_tx {
                let _ = tx.try_send(bytes.clone());
            }
            if let Ok(text) = std::str::from_utf8(bytes) {
                if let Some(port) = detect_dev_server_port(text) {
                    self.detected_port = Some(port);
                }
            }
            self.scan_osc(bytes);
        }

        if let Some(child) = self.child.as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                self.status = format!("Shell exited: {status:?}");
            }
        }

        received_output
    }

    pub(crate) fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.max(6);
        let cols = cols.max(20);

        if rows == self.rows && cols == self.cols {
            return;
        }

        self.rows = rows;
        self.cols = cols;
        self.parser.screen_mut().set_size(rows, cols);

        if let Some(master) = self.master.as_ref() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }

    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    pub(crate) fn point_is_before(a: TerminalPoint, b: TerminalPoint) -> bool {
        a.row < b.row || (a.row == b.row && a.col <= b.col)
    }

    pub(crate) fn normalized_selection(&self) -> Option<(TerminalPoint, TerminalPoint)> {
        let (anchor, focus) = self.selection?;
        if Self::point_is_before(anchor, focus) {
            Some((anchor, focus))
        } else {
            Some((focus, anchor))
        }
    }

    pub(crate) fn select_all(&mut self) {
        self.selection = Some((
            TerminalPoint { row: 0, col: 0 },
            TerminalPoint {
                row: self.rows.saturating_sub(1),
                col: self.cols.saturating_sub(1),
            },
        ));
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        let (start, end) = self.normalized_selection()?;
        let end_col = end.col.saturating_add(1).min(self.cols);
        let text = self
            .parser
            .screen()
            .contents_between(start.row, start.col, end.row, end_col);

        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub(crate) fn copy_selection(&self, ctx: &egui::Context) -> bool {
        if let Some(text) = self.selected_text() {
            ctx.copy_text(text);
            return true;
        }

        false
    }

    pub(crate) fn paste_text(&mut self, text: &str) {
        self.selection = None;
        self.set_scrollback(0);
        if self.parser.screen().bracketed_paste() {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.write_bytes(&bytes);
        } else {
            self.write_bytes(text.as_bytes());
        }
    }

    pub(crate) fn cell_from_pos(
        &self,
        rect: egui::Rect,
        pointer_pos: egui::Pos2,
        char_width: f32,
        row_height: f32,
        padding: f32,
    ) -> TerminalPoint {
        let x = (pointer_pos.x - rect.left() - padding).max(0.0);
        let y = (pointer_pos.y - rect.top() - padding).max(0.0);

        let col = (x / char_width).floor() as u16;
        let row = (y / row_height).floor() as u16;

        TerminalPoint {
            row: row.min(self.rows.saturating_sub(1)),
            col: col.min(self.cols.saturating_sub(1)),
        }
    }

    pub(crate) fn cell_selected(&self, row: u16, col: u16) -> bool {
        let Some((start, end)) = self.normalized_selection() else {
            return false;
        };

        if row < start.row || row > end.row {
            return false;
        }

        if start.row == end.row {
            return row == start.row && col >= start.col && col <= end.col;
        }

        if row == start.row {
            return col >= start.col;
        }

        if row == end.row {
            return col <= end.col;
        }

        true
    }

    pub(crate) fn handle_input(&mut self, ctx: &egui::Context) {
        if !self.has_focus {
            return;
        }

        let events = ctx.input(|input| input.events.clone());
        for event in events {
            match event {
                egui::Event::Text(text) => {
                    if !text.chars().all(char::is_control) {
                        self.selection = None;
                        self.set_scrollback(0);
                        self.write_bytes(text.as_bytes());
                    }
                }
                egui::Event::Paste(text) => {
                    self.pending_logs.push(format!(
                        "img_paste: Event::Paste fired, text.len()={}",
                        text.len()
                    ));
                    if !text.is_empty() {
                        self.paste_text(&text);
                    } else {
                        self.pending_logs
                            .push("img_paste: text empty, trying save_clipboard_image".to_owned());
                        if let Some(img_path) = save_clipboard_image(&mut self.pending_logs) {
                            let filename = img_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("image")
                                .to_owned();
                            let path_str = if let Some(home) = std::env::var_os("HOME") {
                                let abs = img_path.to_string_lossy();
                                let home_str = home.to_string_lossy();
                                if abs.starts_with(home_str.as_ref()) {
                                    format!("~{}", &abs[home_str.len()..])
                                } else {
                                    abs.into_owned()
                                }
                            } else {
                                img_path.to_string_lossy().into_owned()
                            };
                            self.pending_logs
                                .push(format!("img_paste: pasting path: {path_str}"));
                            self.paste_chip = Some(filename);
                            self.paste_text(&path_str);
                        } else {
                            self.pending_logs
                                .push("img_paste: save_clipboard_image returned None".to_owned());
                        }
                    }
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if modifiers.command {
                        match key {
                            egui::Key::C => {
                                if self.copy_selection(ctx) {
                                    continue;
                                }
                            }
                            egui::Key::A => {
                                self.select_all();
                                continue;
                            }
                            egui::Key::V => {
                                self.pending_logs
                                    .push("img_paste: Key::V handler fired".to_owned());
                                let text = read_clipboard().filter(|t| !t.is_empty());
                                if let Some(t) = text {
                                    self.paste_text(&t);
                                } else {
                                    self.pending_logs.push(
                                        "img_paste: no text in clipboard, trying image".to_owned(),
                                    );
                                    if let Some(img_path) =
                                        save_clipboard_image(&mut self.pending_logs)
                                    {
                                        let filename = img_path
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("image")
                                            .to_owned();
                                        let path_str =
                                            if let Some(home) = std::env::var_os("HOME") {
                                                let abs = img_path.to_string_lossy();
                                                let home_str = home.to_string_lossy();
                                                if abs.starts_with(home_str.as_ref()) {
                                                    format!("~{}", &abs[home_str.len()..])
                                                } else {
                                                    abs.into_owned()
                                                }
                                            } else {
                                                img_path.to_string_lossy().into_owned()
                                            };
                                        self.pending_logs
                                            .push(format!("img_paste: pasting path: {path_str}"));
                                        self.paste_chip = Some(filename);
                                        self.paste_text(&path_str);
                                    }
                                }
                                continue;
                            }
                            egui::Key::Backspace
                            | egui::Key::ArrowLeft
                            | egui::Key::ArrowRight => {}
                            _ => continue,
                        }
                    }

                    if let Some(bytes) = self.key_bytes(key, modifiers) {
                        self.selection = None;
                        self.set_scrollback(0);
                        self.write_bytes(&bytes);
                    }
                }
                egui::Event::Copy => {
                    self.copy_selection(ctx);
                }
                _ => {}
            }
        }
    }

    pub(crate) fn key_bytes(&self, key: egui::Key, modifiers: egui::Modifiers) -> Option<Vec<u8>> {
        let application_cursor = self.parser.screen().application_cursor();

        if modifiers.command {
            let command_bytes = match key {
                egui::Key::Backspace => Some(vec![0x15]),
                egui::Key::ArrowLeft => Some(vec![0x01]),
                egui::Key::ArrowRight => Some(vec![0x05]),
                _ => None,
            };

            if let Some(bytes) = command_bytes {
                return Some(bytes);
            }
        }

        if modifiers.alt {
            let alt_bytes = match key {
                egui::Key::Backspace => Some(vec![0x17]),
                egui::Key::ArrowLeft => Some(b"\x1bb".to_vec()),
                egui::Key::ArrowRight => Some(b"\x1bf".to_vec()),
                _ => None,
            };

            if let Some(bytes) = alt_bytes {
                return Some(bytes);
            }
        }

        if modifiers.ctrl {
            let ctrl_byte = match key {
                egui::Key::A => Some(0x01),
                egui::Key::B => Some(0x02),
                egui::Key::C => Some(0x03),
                egui::Key::D => Some(0x04),
                egui::Key::E => Some(0x05),
                egui::Key::F => Some(0x06),
                egui::Key::H => Some(0x08),
                egui::Key::K => Some(0x0B),
                egui::Key::L => Some(0x0C),
                egui::Key::N => Some(0x0E),
                egui::Key::P => Some(0x10),
                egui::Key::U => Some(0x15),
                egui::Key::W => Some(0x17),
                egui::Key::Z => Some(0x1A),
                _ => None,
            };

            if let Some(byte) = ctrl_byte {
                return Some(vec![byte]);
            }
        }

        let bytes = match key {
            egui::Key::Enter => {
                if modifiers.shift {
                    b"\n".to_vec()
                } else {
                    b"\r".to_vec()
                }
            }
            egui::Key::Tab => {
                if modifiers.shift {
                    b"\x1b[Z".to_vec()
                } else {
                    b"\t".to_vec()
                }
            }
            egui::Key::Backspace => vec![0x7F],
            egui::Key::Escape => vec![0x1B],
            egui::Key::ArrowUp => {
                if application_cursor {
                    b"\x1bOA".to_vec()
                } else {
                    b"\x1b[A".to_vec()
                }
            }
            egui::Key::ArrowDown => {
                if application_cursor {
                    b"\x1bOB".to_vec()
                } else {
                    b"\x1b[B".to_vec()
                }
            }
            egui::Key::ArrowRight => {
                if application_cursor {
                    b"\x1bOC".to_vec()
                } else {
                    b"\x1b[C".to_vec()
                }
            }
            egui::Key::ArrowLeft => {
                if application_cursor {
                    b"\x1bOD".to_vec()
                } else {
                    b"\x1b[D".to_vec()
                }
            }
            egui::Key::Home => {
                if application_cursor {
                    b"\x1bOH".to_vec()
                } else {
                    b"\x1b[H".to_vec()
                }
            }
            egui::Key::End => {
                if application_cursor {
                    b"\x1bOF".to_vec()
                } else {
                    b"\x1b[F".to_vec()
                }
            }
            egui::Key::Insert => b"\x1b[2~".to_vec(),
            egui::Key::Delete => b"\x1b[3~".to_vec(),
            egui::Key::PageUp => b"\x1b[5~".to_vec(),
            egui::Key::PageDown => b"\x1b[6~".to_vec(),
            _ => return None,
        };

        Some(bytes)
    }
}

impl Drop for TerminalPane {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
    }
}

pub(crate) fn default_terminal_cwd() -> PathBuf {
    let home_dir = std::env::var_os("HOME").map(PathBuf::from);

    let current_dir = std::env::current_dir().ok();
    if let Some(dir) = current_dir {
        let launched_from_bundle = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent == dir))
            .unwrap_or(false);

        if dir != PathBuf::from("/") && !launched_from_bundle {
            return dir;
        }
    }

    home_dir.unwrap_or_else(|| PathBuf::from("."))
}

// ── Helper free functions ──

pub(crate) fn detect_dev_server_port(text: &str) -> Option<u16> {
    let patterns = [
        "Listening on :",
        "localhost:",
        "127.0.0.1:",
        "Server running on port ",
        "started on port ",
        "running on port ",
        "listening on port ",
    ];
    for pat in patterns {
        if let Some(pos) = text.to_lowercase().find(&pat.to_lowercase()) {
            let rest = &text[pos + pat.len()..];
            let port_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(port) = port_str.parse::<u16>() {
                if port > 0 {
                    return Some(port);
                }
            }
        }
    }
    None
}

pub(crate) fn shell_escape_path(path: &std::path::Path) -> String {
    let display = path.to_string_lossy();
    let escaped = display.replace('\'', "'\\''");
    format!("'{escaped}'")
}

/// Decode `%XX` escapes in an OSC 7 file URL path.
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Directory holding the injected zsh startup files for shell integration.
fn shell_init_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("StickyTerminal")
            .join("shell-init"),
    )
}

/// Write the temp ZDOTDIR files that emit OSC 133 (command boundaries) and
/// OSC 7 (cwd). Each file first sources the user's real equivalent so their
/// environment is untouched. Idempotent — rewritten every launch. Returns the
/// directory to point `ZDOTDIR` at, or `None` if it can't be created.
fn ensure_zsh_integration() -> Option<PathBuf> {
    let dir = shell_init_dir()?;
    std::fs::create_dir_all(&dir).ok()?;

    let source_user = |name: &str| {
        format!("[ -f \"${{USER_ZDOTDIR:-$HOME}}/{name}\" ] && . \"${{USER_ZDOTDIR:-$HOME}}/{name}\"\n")
    };

    let zshrc = format!(
        "{}\n# StickyTerminal shell integration — OSC 133 command boundaries + OSC 7 cwd.\n\
         __sticky_precmd() {{\n\
         \x20 local ec=$?\n\
         \x20 printf '\\033]133;D;%s\\007' \"$ec\"\n\
         \x20 printf '\\033]7;file://%s%s\\007' \"${{HOST}}\" \"${{PWD}}\"\n\
         \x20 printf '\\033]133;A\\007'\n\
         }}\n\
         __sticky_preexec() {{ printf '\\033]133;C\\007'; }}\n\
         autoload -Uz add-zsh-hook 2>/dev/null\n\
         if (( $+functions[add-zsh-hook] )); then\n\
         \x20 add-zsh-hook precmd __sticky_precmd\n\
         \x20 add-zsh-hook preexec __sticky_preexec\n\
         fi\n\
         ZDOTDIR=\"${{USER_ZDOTDIR:-$HOME}}\"\n",
        source_user(".zshrc"),
    );

    std::fs::write(dir.join(".zshenv"), source_user(".zshenv")).ok()?;
    std::fs::write(dir.join(".zprofile"), source_user(".zprofile")).ok()?;
    std::fs::write(dir.join(".zlogin"), source_user(".zlogin")).ok()?;
    std::fs::write(dir.join(".zshrc"), zshrc).ok()?;

    Some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn detects_dev_server_ports_from_common_messages() {
        assert_eq!(detect_dev_server_port("Listening on :3000"), Some(3000));
        assert_eq!(detect_dev_server_port("ready at http://localhost:8787"), Some(8787));
        assert_eq!(detect_dev_server_port("Server running on port 5173"), Some(5173));
    }

    #[test]
    fn ignores_missing_or_invalid_ports() {
        assert_eq!(detect_dev_server_port("Listening on :0"), None);
        assert_eq!(detect_dev_server_port("no local server here"), None);
    }

    #[test]
    fn percent_decode_handles_escapes() {
        assert_eq!(percent_decode("/Users/me/My%20Code"), "/Users/me/My Code");
        assert_eq!(percent_decode("/plain/path"), "/plain/path");
    }

    #[test]
    fn scan_osc_tracks_command_status() {
        let mut pane = TerminalPane::new(PathBuf::from("/"));
        pane.scan_osc(b"\x1b]133;C\x07");
        assert!(pane.command_running);
        pane.scan_osc(b"output\x1b]133;D;0\x07more");
        assert!(!pane.command_running);
        assert_eq!(pane.last_exit, Some(0));
        pane.scan_osc(b"\x1b]133;D;1\x07");
        assert_eq!(pane.last_exit, Some(1));
    }

    #[test]
    fn scan_osc_reassembles_split_sequence() {
        let mut pane = TerminalPane::new(PathBuf::from("/"));
        pane.scan_osc(b"\x1b]133;D");
        pane.scan_osc(b";0\x07");
        assert_eq!(pane.last_exit, Some(0));
    }

    #[test]
    fn scan_osc_reads_cwd_from_osc7() {
        let mut pane = TerminalPane::new(PathBuf::from("/"));
        pane.scan_osc(b"\x1b]7;file://host/Users/me/proj\x07");
        assert_eq!(pane.live_cwd, Some(PathBuf::from("/Users/me/proj")));
    }

    #[test]
    fn escapes_shell_paths_with_single_quotes() {
        assert_eq!(
            shell_escape_path(Path::new("/tmp/it's here")),
            "'/tmp/it'\\''s here'"
        );
    }
}
