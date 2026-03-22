use eframe::egui;
use std::path::Path;

use crate::terminal::{AiLineCategory, AiOutputPane, BrowserPane, Pane, TerminalPane};
use crate::terminal::shell_escape_path;
use crate::theme::ThemePalette;

pub(crate) fn render_pane(
    pane: &mut TerminalPane,
    ui: &mut egui::Ui,
    palette: ThemePalette,
    ctx: &egui::Context,
    pane_id: egui::Id,
    is_active: bool,
) {
    const DROP_TARGET_ID: &str = "terminal_drop_target";
    const SCROLLBAR_WIDTH: f32 = 10.0;
    const SCROLLBAR_GAP: f32 = 6.0;

    let frame = egui::Frame::NONE
        .fill(palette.terminal_bg)
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(6));

    frame.show(ui, |ui| {
        let terminal_id = pane_id.with("terminal_surface");
        let drop_target_id = egui::Id::new(DROP_TARGET_ID);
        let size = ui.available_size();
        let (_, rect) = ui.allocate_space(size);
        let content_rect = egui::Rect::from_min_max(
            rect.min,
            egui::pos2(
                (rect.max.x - SCROLLBAR_WIDTH - SCROLLBAR_GAP).max(rect.min.x),
                rect.max.y,
            ),
        );
        let scrollbar_rect = egui::Rect::from_min_max(
            egui::pos2(content_rect.max.x + SCROLLBAR_GAP, rect.top() + 4.0),
            egui::pos2(rect.right(), rect.bottom() - 4.0),
        );
        let response = ui.interact(content_rect, terminal_id, egui::Sense::click_and_drag());
        let scrollbar_response = ui.interact(
            scrollbar_rect,
            pane_id.with("terminal_scrollbar"),
            egui::Sense::click_and_drag(),
        );
        let hovered_files = ctx.input(|input| input.raw.hovered_files.clone());
        let dropped_files = ctx.input(|input| input.raw.dropped_files.clone());

        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let measure =
            ui.painter()
                .layout_no_wrap("W".to_owned(), font_id.clone(), palette.text);
        let char_width = measure.size().x.max(8.0);
        let row_height = measure.size().y.max(16.0) + 2.0;
        let inner_padding = 4.0;

        let rows = ((content_rect.height() - inner_padding * 2.0) / row_height).floor() as u16;
        let cols = ((content_rect.width() - inner_padding * 2.0) / char_width).floor() as u16;

        pane.resize(rows, cols);
        let max_scrollback = pane.max_scrollback();

        let cmd_held = ctx.input(|i| i.modifiers.command);

        if response.clicked() {
            let mut url_opened = false;
            if cmd_held {
                if let Some(pos) = response.interact_pointer_pos() {
                    let point = pane.cell_from_pos(
                        content_rect,
                        pos,
                        char_width,
                        row_height,
                        inner_padding,
                    );
                    let url_to_open = {
                        let screen = pane.parser.screen();
                        let spans = find_row_url_spans(screen, point.row, pane.cols);
                        spans
                            .into_iter()
                            .find(|(s, e, _)| point.col >= *s && point.col <= *e)
                            .map(|(_, _, url)| url)
                    };
                    if let Some(url) = url_to_open {
                        open_url(&url);
                        url_opened = true;
                    }
                }
            }
            if !url_opened {
                response.request_focus();
                pane.selection = None;
            }
        }

        if response.drag_started() {
            response.request_focus();
            if let Some(pointer_pos) = response.interact_pointer_pos() {
                let point = pane.cell_from_pos(
                    content_rect,
                    pointer_pos,
                    char_width,
                    row_height,
                    inner_padding,
                );
                pane.selection = Some((point, point));
            }
        }

        if response.dragged() {
            if let Some(pointer_pos) = response.interact_pointer_pos() {
                let point = pane.cell_from_pos(
                    content_rect,
                    pointer_pos,
                    char_width,
                    row_height,
                    inner_padding,
                );
                if let Some((anchor, _)) = pane.selection {
                    pane.selection = Some((anchor, point));
                }
            }
        }

        if response.hovered() || scrollbar_response.hovered() {
            let scroll_delta = ctx.input(|input| input.smooth_scroll_delta.y);
            if scroll_delta.abs() > f32::EPSILON {
                let rows_delta = (scroll_delta / row_height).round() as i32;
                if rows_delta != 0 {
                    pane.adjust_scrollback(rows_delta);
                }
            }
        }

        if !hovered_files.is_empty() && response.hovered() {
            ui.data_mut(|data| data.insert_temp(drop_target_id, Some(pane_id)));
        }

        let is_drop_target = ui
            .data(|data| data.get_temp::<Option<egui::Id>>(drop_target_id).flatten())
            == Some(pane_id);

        if is_drop_target && !dropped_files.is_empty() {
            let dropped_paths = dropped_files
                .iter()
                .filter_map(|file| {
                    file.path
                        .as_ref()
                        .map(|path| shell_escape_path(path))
                        .or_else(|| {
                            (!file.name.is_empty())
                                .then(|| shell_escape_path(Path::new(&file.name)))
                        })
                })
                .collect::<Vec<_>>();
            if !dropped_paths.is_empty() {
                response.request_focus();
                let pasted_paths = dropped_paths.join(" ");
                ctx.copy_text(pasted_paths.clone());
                pane.paste_text(&(pasted_paths + " "));
                pane.status = if dropped_paths.len() == 1 {
                    "Dropped path copied and pasted.".to_owned()
                } else {
                    format!("{} dropped paths copied and pasted.", dropped_paths.len())
                };
            }
            ui.data_mut(|data| {
                data.remove_temp::<Option<egui::Id>>(drop_target_id);
            });
        } else if hovered_files.is_empty() && dropped_files.is_empty() && is_drop_target {
            ui.data_mut(|data| {
                data.remove_temp::<Option<egui::Id>>(drop_target_id);
            });
        }

        pane.has_focus = ui.memory(|memory| memory.has_focus(terminal_id));
        if pane.has_focus {
            ui.memory_mut(|memory| {
                memory.set_focus_lock_filter(
                    terminal_id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
        }
        pane.handle_input(ctx);

        let painter = ui.painter_at(rect);

        if is_active {
            painter.rect(
                rect.shrink(0.5),
                egui::CornerRadius::same(6),
                egui::Color32::TRANSPARENT,
                egui::Stroke::new(1.0, palette.accent.linear_multiply(0.3)),
                egui::StrokeKind::Inside,
            );
        }

        if max_scrollback > 0 {
            let scrollback_offset = pane.scrollback_position();
            let total_rows = max_scrollback + usize::from(pane.rows.max(1));
            let visible_ratio = pane.rows as f32 / total_rows as f32;
            let thumb_height =
                (scrollbar_rect.height() * visible_ratio).clamp(28.0, scrollbar_rect.height());

            painter.rect_filled(
                scrollbar_rect,
                egui::CornerRadius::same(5),
                palette.surface.linear_multiply(0.55),
            );

            let travel = (scrollbar_rect.height() - thumb_height).max(0.0);
            let thumb_top = if travel <= f32::EPSILON {
                scrollbar_rect.top()
            } else {
                scrollbar_rect.top()
                    + (1.0 - scrollback_offset as f32 / max_scrollback as f32) * travel
            };
            let thumb_rect = egui::Rect::from_min_size(
                egui::pos2(scrollbar_rect.left(), thumb_top),
                egui::vec2(scrollbar_rect.width(), thumb_height),
            );
            let thumb_color = if scrollbar_response.dragged() {
                palette.accent
            } else if scrollbar_response.hovered() || scrollback_offset > 0 {
                palette.text.linear_multiply(0.72)
            } else {
                palette.muted_text.linear_multiply(0.5)
            };
            painter.rect_filled(thumb_rect, egui::CornerRadius::same(5), thumb_color);

            if let Some(pointer_pos) = scrollbar_response.interact_pointer_pos() {
                if scrollbar_response.clicked() || scrollbar_response.dragged() {
                    let travel = (scrollbar_rect.height() - thumb_height).max(1.0);
                    let thumb_y = (pointer_pos.y - scrollbar_rect.top() - thumb_height * 0.5)
                        .clamp(0.0, travel);
                    let top_ratio = thumb_y / travel;
                    pane.set_scrollback(
                        ((1.0 - top_ratio) * max_scrollback as f32).round() as usize,
                    );
                }
            }
        }

        let screen = pane.parser.screen();

        let hovered_cell = if cmd_held && response.hovered() {
            ctx.input(|i| i.pointer.hover_pos())
                .filter(|p| content_rect.contains(*p))
                .map(|p| pane.cell_from_pos(content_rect, p, char_width, row_height, inner_padding))
        } else {
            None
        };
        let mut set_hand_cursor = false;

        for row in 0..pane.rows {
            let url_spans: Vec<(u16, u16, String)> = if cmd_held {
                find_row_url_spans(screen, row, pane.cols)
            } else {
                Vec::new()
            };

            for col in 0..pane.cols {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };

                if cell.is_wide_continuation() {
                    continue;
                }

                let mut fg = resolve_terminal_color(cell.fgcolor(), palette.text);
                let mut bg = resolve_terminal_color(cell.bgcolor(), palette.terminal_bg);

                if cell.inverse() {
                    std::mem::swap(&mut fg, &mut bg);
                }

                if cell.dim() {
                    fg = fg.linear_multiply(0.7);
                }

                let cell_width = if cell.is_wide() {
                    char_width * 2.0
                } else {
                    char_width
                };
                let min = egui::pos2(
                    content_rect.left() + inner_padding + col as f32 * char_width,
                    content_rect.top() + inner_padding + row as f32 * row_height,
                );
                let cell_rect = egui::Rect::from_min_size(
                    min,
                    egui::vec2(cell_width.max(char_width), row_height),
                );

                if !matches!(cell.bgcolor(), vt100::Color::Default) || cell.inverse() {
                    painter.rect_filled(cell_rect, egui::CornerRadius::ZERO, bg);
                }

                if pane.cell_selected(row, col) {
                    painter.rect_filled(cell_rect, egui::CornerRadius::ZERO, palette.selection);
                }

                if !cell.has_contents() {
                    continue;
                }

                let mut draw_font = font_id.clone();
                if cell.bold() {
                    draw_font.size += 1.0;
                }

                painter.text(min, egui::Align2::LEFT_TOP, cell.contents(), draw_font, fg);

                if cell.underline() {
                    let y = cell_rect.bottom() - 3.0;
                    painter.line_segment(
                        [
                            egui::pos2(cell_rect.left(), y),
                            egui::pos2(cell_rect.right(), y),
                        ],
                        egui::Stroke::new(1.0, fg),
                    );
                }

                if let Some((us, ue, _)) =
                    url_spans.iter().find(|(s, e, _)| col >= *s && col <= *e)
                {
                    let is_hovered = hovered_cell
                        .map(|hp| hp.row == row && hp.col >= *us && hp.col <= *ue)
                        .unwrap_or(false);
                    if is_hovered {
                        set_hand_cursor = true;
                    }
                    let ucolor = if is_hovered {
                        palette.accent
                    } else {
                        palette.accent.linear_multiply(0.5)
                    };
                    let y = cell_rect.bottom() - 2.0;
                    painter.line_segment(
                        [
                            egui::pos2(cell_rect.left(), y),
                            egui::pos2(cell_rect.right(), y),
                        ],
                        egui::Stroke::new(1.0, ucolor),
                    );
                }
            }
        }

        if set_hand_cursor {
            ctx.output_mut(|o| o.cursor_icon = egui::CursorIcon::PointingHand);
        }

        let (cursor_row, cursor_col) = screen.cursor_position();
        if pane.has_focus {
            let x = content_rect.left() + inner_padding + cursor_col as f32 * char_width;
            let y = content_rect.top() + inner_padding + cursor_row as f32 * row_height;
            let cursor_rect = egui::Rect::from_min_size(
                egui::pos2(x, y),
                egui::vec2(2.0, (row_height - 2.0).max(12.0)),
            );
            painter.rect_filled(cursor_rect, egui::CornerRadius::same(1), palette.accent);
        } else {
            painter.text(
                content_rect.right_top() + egui::vec2(-10.0, 6.0),
                egui::Align2::RIGHT_TOP,
                "click to focus",
                egui::TextStyle::Small.resolve(ui.style()),
                palette.muted_text,
            );
        }

        if is_drop_target && !hovered_files.is_empty() {
            painter.rect_filled(
                rect,
                egui::CornerRadius::same(8),
                palette.selection.linear_multiply(0.3),
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop files or folders to paste their paths",
                egui::TextStyle::Button.resolve(ui.style()),
                palette.text,
            );
        }

        let chip_dismiss = if let Some(ref filename) = pane.paste_chip {
            let font = egui::FontId::proportional(12.0);
            let label = format!("\u{1F5BC} {filename}");
            let text_width = painter
                .layout_no_wrap(label.clone(), font.clone(), egui::Color32::WHITE)
                .size()
                .x;
            let chip_w = text_width + 20.0 + 20.0;
            let chip_h = 26.0;
            let chip_margin = 8.0;
            let chip_rect = egui::Rect::from_min_size(
                egui::pos2(
                    rect.left() + chip_margin,
                    rect.bottom() - chip_margin - chip_h,
                ),
                egui::vec2(chip_w, chip_h),
            );

            let chip_bg = egui::Color32::from_rgb(40, 42, 56);
            let chip_border = egui::Color32::from_rgb(100, 105, 140);
            let text_color = egui::Color32::from_rgb(210, 215, 235);
            let close_color = egui::Color32::from_rgb(160, 160, 180);

            painter.rect(
                chip_rect,
                egui::CornerRadius::same(6),
                chip_bg,
                egui::Stroke::new(1.0, chip_border),
                egui::StrokeKind::Outside,
            );
            painter.text(
                egui::pos2(chip_rect.left() + 8.0, chip_rect.center().y),
                egui::Align2::LEFT_CENTER,
                &label,
                font,
                text_color,
            );

            let close_rect = egui::Rect::from_min_size(
                egui::pos2(chip_rect.right() - 22.0, chip_rect.top()),
                egui::vec2(22.0, chip_h),
            );
            let close_id = pane_id.with("paste_chip_close");
            let close_resp = ui.interact(close_rect, close_id, egui::Sense::click());
            let x_color = if close_resp.hovered() {
                egui::Color32::from_rgb(220, 80, 80)
            } else {
                close_color
            };
            painter.text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                "×",
                egui::FontId::proportional(14.0),
                x_color,
            );
            close_resp.clicked()
        } else {
            false
        };
        if chip_dismiss {
            pane.paste_chip = None;
        }
    });
}

pub(crate) fn render_ai_output_pane(
    pane: &mut AiOutputPane,
    ui: &mut egui::Ui,
    palette: ThemePalette,
    is_active: bool,
) {
    let frame = egui::Frame::NONE
        .fill(palette.terminal_bg)
        .stroke(egui::Stroke::new(
            if is_active { 1.5 } else { 0.5 },
            if is_active { palette.accent } else { palette.border },
        ))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(0));

    frame.show(ui, |ui| {
        let title_rect =
            egui::Rect::from_min_size(ui.min_rect().min, egui::vec2(ui.available_width(), 24.0));
        ui.allocate_rect(title_rect, egui::Sense::hover());
        ui.painter()
            .rect_filled(title_rect, egui::CornerRadius::same(6), palette.tab_bg);
        ui.painter().text(
            title_rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("AI: {}", pane.title),
            egui::FontId::proportional(12.0),
            palette.text,
        );

        let content_rect = egui::Rect::from_min_max(
            egui::pos2(ui.min_rect().min.x, ui.min_rect().min.y + 24.0),
            ui.min_rect().max,
        );
        ui.allocate_rect(content_rect, egui::Sense::hover());

        let mut child_ui =
            ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
        egui::ScrollArea::vertical()
            .id_salt(format!("ai_scroll_{}", pane.uid))
            .stick_to_bottom(true)
            .show(&mut child_ui, |ui| {
                for line in &pane.lines {
                    let color = match line.cat {
                        AiLineCategory::Tool => egui::Color32::from_rgb(100, 200, 255),
                        AiLineCategory::Edit => egui::Color32::from_rgb(100, 255, 150),
                        AiLineCategory::Bash => egui::Color32::from_rgb(255, 220, 100),
                        AiLineCategory::Error => egui::Color32::from_rgb(255, 100, 100),
                        AiLineCategory::Normal => palette.text,
                    };
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(&line.text)
                                .color(color)
                                .monospace()
                                .size(12.0),
                        )
                        .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                }
            });
    });
}

pub(crate) fn render_browser_pane(
    pane: &mut BrowserPane,
    ui: &mut egui::Ui,
    palette: ThemePalette,
) {
    let frame = egui::Frame::NONE
        .fill(egui::Color32::from_rgb(30, 30, 36))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(6));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("URL:").color(palette.muted_text).small());
            ui.add(
                egui::TextEdit::singleline(&mut pane.url_bar)
                    .desired_width(ui.available_width() - 60.0)
                    .hint_text("https://..."),
            );
            if ui.button("Go").clicked() {
                pane.current_url = if pane.url_bar.starts_with("http://")
                    || pane.url_bar.starts_with("https://")
                    || pane.url_bar.starts_with("about:")
                {
                    pane.url_bar.clone()
                } else {
                    format!("https://{}", pane.url_bar)
                };
            }
        });
        let rect = ui.available_rect_before_wrap();
        ui.allocate_rect(rect, egui::Sense::hover());
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(4),
            egui::Color32::from_rgb(20, 20, 26),
        );
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            format!("Browser\n{}", pane.current_url),
            egui::FontId::proportional(14.0),
            palette.muted_text,
        );
    });
}

/// Dispatch rendering to the correct pane type
pub(crate) fn render_pane_dispatch(
    pane: &mut Pane,
    ui: &mut egui::Ui,
    palette: ThemePalette,
    ctx: &egui::Context,
    pane_id: egui::Id,
    is_active: bool,
) {
    match pane {
        Pane::Terminal(t) => render_pane(t, ui, palette, ctx, pane_id, is_active),
        Pane::AiOutput(a) => render_ai_output_pane(a, ui, palette, is_active),
        Pane::Browser(b) => render_browser_pane(b, ui, palette),
    }
}

// ── URL / helper functions ──

pub(crate) fn find_row_url_spans(
    screen: &vt100::Screen,
    row: u16,
    cols: u16,
) -> Vec<(u16, u16, String)> {
    let mut char_to_col: Vec<u16> = Vec::with_capacity(cols as usize);
    let mut row_str = String::with_capacity(cols as usize);
    for col in 0..cols {
        if let Some(cell) = screen.cell(row, col) {
            if cell.is_wide_continuation() {
                continue;
            }
            let content = cell.contents();
            if content.is_empty() {
                char_to_col.push(col);
                row_str.push(' ');
            } else {
                for ch in content.chars() {
                    char_to_col.push(col);
                    row_str.push(ch);
                }
            }
        } else {
            char_to_col.push(col);
            row_str.push(' ');
        }
    }

    let mut spans: Vec<(u16, u16, String)> = Vec::new();
    let mut search_from = 0usize;
    loop {
        let found = ["https://", "http://", "ftp://"]
            .iter()
            .filter_map(|p| {
                row_str[search_from..]
                    .find(p)
                    .map(|pos| (search_from + pos, *p))
            })
            .min_by_key(|(pos, _)| *pos);
        let Some((abs_start, prefix)) = found else {
            break;
        };
        let url_tail = &row_str[abs_start..];
        let url_end = url_tail
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ')' | ']' | '>' | '<'))
            .unwrap_or(url_tail.len());
        if url_end > prefix.len() {
            let url = url_tail[..url_end].to_string();
            let start_col = char_to_col.get(abs_start).copied().unwrap_or(0);
            let end_col = char_to_col
                .get(abs_start + url_end - 1)
                .copied()
                .unwrap_or(start_col);
            spans.push((start_col, end_col, url));
            search_from = abs_start + url_end;
        } else {
            search_from = abs_start + prefix.len();
        }
    }
    spans
}

pub(crate) fn open_url(url: &str) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

fn ansi_index_color(index: u8) -> egui::Color32 {
    match index {
        0 => egui::Color32::from_rgb(0, 0, 0),
        1 => egui::Color32::from_rgb(205, 49, 49),
        2 => egui::Color32::from_rgb(13, 188, 121),
        3 => egui::Color32::from_rgb(229, 229, 16),
        4 => egui::Color32::from_rgb(36, 114, 200),
        5 => egui::Color32::from_rgb(188, 63, 188),
        6 => egui::Color32::from_rgb(17, 168, 205),
        7 => egui::Color32::from_rgb(229, 229, 229),
        8 => egui::Color32::from_rgb(102, 102, 102),
        9 => egui::Color32::from_rgb(241, 76, 76),
        10 => egui::Color32::from_rgb(35, 209, 139),
        11 => egui::Color32::from_rgb(245, 245, 67),
        12 => egui::Color32::from_rgb(59, 142, 234),
        13 => egui::Color32::from_rgb(214, 112, 214),
        14 => egui::Color32::from_rgb(41, 184, 219),
        15 => egui::Color32::from_rgb(255, 255, 255),
        16..=231 => {
            let value = index - 16;
            let r = value / 36;
            let g = (value % 36) / 6;
            let b = value % 6;
            let channel = |component: u8| {
                if component == 0 {
                    0
                } else {
                    55 + component * 40
                }
            };
            egui::Color32::from_rgb(channel(r), channel(g), channel(b))
        }
        232..=255 => {
            let level = 8 + (index - 232) * 10;
            egui::Color32::from_rgb(level, level, level)
        }
    }
}

pub(crate) fn resolve_terminal_color(
    color: vt100::Color,
    default_color: egui::Color32,
) -> egui::Color32 {
    match color {
        vt100::Color::Default => default_color,
        vt100::Color::Idx(index) => ansi_index_color(index),
        vt100::Color::Rgb(r, g, b) => egui::Color32::from_rgb(r, g, b),
    }
}
