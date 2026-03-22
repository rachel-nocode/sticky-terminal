pub(crate) mod clipboard;

use eframe::egui;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use vt100::Parser;

use crate::notes::ParsedMarkdownLine;
use crate::TERMINAL_SCROLLBACK;
pub(crate) use clipboard::{install_paste_monitor, read_clipboard, save_clipboard_image, CMD_V_PRESSED};

#[derive(Clone, Copy)]
pub(crate) struct TerminalPoint {
    pub(crate) row: u16,
    pub(crate) col: u16,
}

// ── A single terminal session (pane) ──
pub(crate) struct TerminalPane {
    pub(crate) uid: u64,
    pub(crate) title: String,
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
    pub(crate) font_scale: f32,
}

// ── A tab containing one or more panes ──
pub(crate) struct TerminalTab {
    pub(crate) title: String,
    pub(crate) panes: Vec<TerminalPane>,
    pub(crate) active_pane: usize,
    pub(crate) notes_markdown: String,
    pub(crate) current_note_file: Option<PathBuf>,
    pub(crate) note_status: String,
    pub(crate) editing_notes: bool,
    pub(crate) notes_dirty: bool,
    pub(crate) last_type_time: Option<std::time::Instant>,
    /// Cache for markdown preview: (content_hash, pre-parsed blocks).
    /// Rebuilt only when the markdown content changes; rendered from each frame.
    pub(crate) notes_render_cache: Option<(u64, Vec<ParsedMarkdownLine>)>,
}

// ── TerminalPane implementation ──

impl TerminalPane {
    pub(crate) fn new(uid: u64, cwd: PathBuf) -> Self {
        let rows = 28;
        let cols = 120;

        Self {
            uid,
            title: String::new(),
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
            font_scale: 1.0,
        }
    }

    pub(crate) fn shell_builder(&self) -> CommandBuilder {
        #[cfg(target_os = "windows")]
        {
            let mut command = CommandBuilder::new("cmd.exe");
            command.cwd(self.cwd.as_os_str());
            command
        }

        #[cfg(not(target_os = "windows"))]
        {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
            let mut command = CommandBuilder::new(shell);
            command.arg("-il");
            command.cwd(self.cwd.as_os_str());
            command.env("TERM", "xterm-256color");
            command.env("COLORTERM", "truecolor");
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

        let (tx, rx) = mpsc::channel();
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
        let mut received_output = false;

        if let Some(rx) = &self.rx {
            while let Ok(bytes) = rx.try_recv() {
                self.parser.process(&bytes);
                received_output = true;
            }
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
                                        let path_str = if let Some(home) = std::env::var_os("HOME")
                                        {
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
                            egui::Key::Backspace | egui::Key::ArrowLeft | egui::Key::ArrowRight => {
                            }
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
                egui::Key::Backspace => Some(vec![0x17]), // Ctrl+W = backward-kill-word
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

pub(crate) fn shell_escape_path(path: &Path) -> String {
    let display = path.to_string_lossy();
    let escaped = display.replace('\'', "'\\''");
    format!("'{escaped}'")
}

// ── TerminalTab implementation ──

impl TerminalTab {
    pub(crate) fn new(number: usize, uid: u64, cwd: PathBuf) -> Self {
        Self {
            title: format!("Tab {number}"),
            panes: vec![TerminalPane::new(uid, cwd)],
            active_pane: 0,
            notes_markdown: "# TODO\n- [ ] Keep this shell usable for Codex CLI\n- [ ] Add quick project tabs\n- [ ] Save notes between sessions\n\n## Notes\nWrite markdown on the left.\nUse the right side like a real terminal.".to_owned(),
            current_note_file: None,
            note_status: "Set your notes folder to start saving notes.".to_owned(),
            editing_notes: false,
            notes_dirty: false,
            last_type_time: None,
            notes_render_cache: None,
        }
    }

    pub(crate) fn active_pane(&self) -> &TerminalPane {
        &self.panes[self.active_pane]
    }

    pub(crate) fn split_pane(&mut self, uid: u64) {
        let cwd = self.panes[self.active_pane].cwd.clone();
        let insert_at = self.active_pane + 1;
        self.panes.insert(insert_at, TerminalPane::new(uid, cwd));
        self.active_pane = insert_at;
    }

    pub(crate) fn close_active_pane(&mut self) -> bool {
        if self.panes.len() <= 1 {
            return false;
        }
        self.panes.remove(self.active_pane);
        if self.active_pane >= self.panes.len() {
            self.active_pane = self.panes.len() - 1;
        }
        true
    }

    pub(crate) fn close_pane(&mut self, idx: usize) -> bool {
        if self.panes.len() <= 1 || idx >= self.panes.len() {
            return false;
        }
        self.panes.remove(idx);
        if self.active_pane >= self.panes.len() {
            self.active_pane = self.panes.len() - 1;
        } else if self.active_pane > idx {
            self.active_pane -= 1;
        }
        true
    }

    pub(crate) fn focus_next_pane(&mut self) {
        if self.panes.len() > 1 {
            self.active_pane = (self.active_pane + 1) % self.panes.len();
        }
    }

    pub(crate) fn focus_prev_pane(&mut self) {
        if self.panes.len() > 1 {
            self.active_pane = if self.active_pane == 0 {
                self.panes.len() - 1
            } else {
                self.active_pane - 1
            };
        }
    }

    pub(crate) fn ensure_all_started(&mut self) {
        for pane in &mut self.panes {
            pane.ensure_started();
        }
    }

    pub(crate) fn drain_all_output(&mut self) -> bool {
        let mut any = false;
        for pane in &mut self.panes {
            if pane.drain_output() {
                any = true;
            }
        }
        any
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
