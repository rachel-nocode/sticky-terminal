use eframe::egui;
use std::path::Path;

use crate::terminal::{shell_escape_path, TerminalPane};
use crate::theme::ThemePalette;
use crate::PANE_SEPARATOR_WIDTH;

use super::GhostStickiesApp;

impl GhostStickiesApp {
    /// Render a single terminal pane
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

            let base_font_id = egui::TextStyle::Monospace.resolve(ui.style());
            let font_id = egui::FontId::monospace(base_font_id.size * pane.font_scale);
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
                            let spans = Self::find_row_url_spans(screen, point.row, pane.cols);
                            spans
                                .into_iter()
                                .find(|(s, e, _)| point.col >= *s && point.col <= *e)
                                .map(|(_, _, url)| url)
                        };
                        if let Some(url) = url_to_open {
                            Self::open_url(&url);
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

            // Draw a subtle active-pane indicator
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
                            ((1.0 - top_ratio) * max_scrollback as f32).round() as usize
                        );
                    }
                }
            }

            let screen = pane.parser.screen();

            // Cell position under the pointer (for URL hover highlight).
            let hovered_cell = if cmd_held && response.hovered() {
                ctx.input(|i| i.pointer.hover_pos())
                    .filter(|p| content_rect.contains(*p))
                    .map(|p| {
                        pane.cell_from_pos(content_rect, p, char_width, row_height, inner_padding)
                    })
            } else {
                None
            };
            let mut set_hand_cursor = false;

            for row in 0..pane.rows {
                // Viewport culling: skip rows outside the visible clip rect
                let row_y_top = content_rect.top() + inner_padding + row as f32 * row_height;
                let row_y_bot = row_y_top + row_height;
                let clip = ui.clip_rect();
                if row_y_bot < clip.top() || row_y_top > clip.bottom() {
                    continue;
                }

                let url_spans: Vec<(u16, u16, String)> = if cmd_held {
                    Self::find_row_url_spans(screen, row, pane.cols)
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

                    let mut fg = Self::resolve_terminal_color(cell.fgcolor(), palette.text);
                    let mut bg = Self::resolve_terminal_color(cell.bgcolor(), palette.terminal_bg);

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

                    // URL underline when Cmd is held.
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

            // ── Paste-image chip overlay ─────────────────────────────────
            let chip_dismiss = if let Some(ref filename) = pane.paste_chip {
                let font = egui::FontId::proportional(12.0);
                let label = format!("\u{1F5BC} {filename}");
                let text_width = painter
                    .layout_no_wrap(label.clone(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x;
                let chip_w = text_width + 20.0 + 20.0; // text + padding + close btn
                let chip_h = 26.0;
                let chip_margin = 8.0;
                // Position at bottom-left so it doesn't cover terminal output
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

                // × close button
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

    /// Compute grid dimensions for n panes
    pub(crate) fn grid_dims(n: usize) -> (usize, usize) {
        match n {
            0 | 1 => (1, 1),
            2 => (2, 1),
            3 => (3, 1),
            4 => (2, 2),
            5 | 6 => (3, 2),
            7..=9 => (3, 3),
            10..=12 => (4, 3),
            _ => {
                let cols = (n as f32).sqrt().ceil() as usize;
                let rows = (n + cols - 1) / cols;
                (cols, rows)
            }
        }
    }

    /// Render all panes in an auto-grid layout with drag-to-swap
    pub(crate) fn render_panes(&mut self, ui: &mut egui::Ui, palette: ThemePalette, ctx: &egui::Context) {
        let tab_idx = self.active_terminal;
        let num_panes = self.terminal_tabs[tab_idx].panes.len();
        let active_pane_idx = self.terminal_tabs[tab_idx].active_pane;

        if num_panes == 1 {
            let pane_uid = self.terminal_tabs[tab_idx].panes[0].uid;
            let pane_id = ui.id().with(("pane_uid", pane_uid));
            let pane = &mut self.terminal_tabs[tab_idx].panes[0];
            Self::render_pane(pane, ui, palette, ctx, pane_id, true);
            let logs = std::mem::take(&mut self.terminal_tabs[tab_idx].panes[0].pending_logs);
            for msg in logs {
                self.log_debug(msg);
            }
            return;
        }

        let (cols, rows) = Self::grid_dims(num_panes);
        let total_width = ui.available_width();
        let total_height = ui.available_height();
        let gap = PANE_SEPARATOR_WIDTH;
        let pane_width = (total_width - gap * (cols as f32 - 1.0)) / cols as f32;
        let pane_height = (total_height - gap * (rows as f32 - 1.0)) / rows as f32;

        // Collect rects for each pane slot to detect drag targets
        let mut pane_rects: Vec<egui::Rect> = Vec::with_capacity(num_panes);
        let origin = ui.cursor().min;

        for idx in 0..num_panes {
            let col = idx % cols;
            let row = idx / cols;
            let x = origin.x + col as f32 * (pane_width + gap);
            let y = origin.y + row as f32 * (pane_height + gap);
            pane_rects.push(egui::Rect::from_min_size(
                egui::pos2(x, y),
                egui::vec2(pane_width, pane_height),
            ));
        }

        // Draw separators
        let painter = ui.painter();
        for row in 0..rows {
            let panes_in_row = if (row + 1) * cols <= num_panes {
                cols
            } else {
                num_panes - row * cols
            };

            // Vertical separators between columns
            for col in 1..panes_in_row {
                let x = origin.x + col as f32 * (pane_width + gap) - gap;
                let y_top = origin.y + row as f32 * (pane_height + gap);
                let sep_rect =
                    egui::Rect::from_min_size(egui::pos2(x, y_top), egui::vec2(gap, pane_height));
                painter.rect_filled(sep_rect, egui::CornerRadius::ZERO, palette.border);
            }

            // Horizontal separator below this row (if not last row)
            if row + 1 < rows {
                let y = origin.y + (row + 1) as f32 * (pane_height + gap) - gap;
                let sep_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x, y),
                    egui::vec2(total_width, gap),
                );
                painter.rect_filled(sep_rect, egui::CornerRadius::ZERO, palette.border);
            }
        }

        const BAR_H: f32 = 24.0;

        // Render each pane in its grid slot
        let mut pending_focus: Option<usize> = None;
        let mut pending_swap: Option<(usize, usize)> = None;
        let mut pending_close: Option<usize> = None;
        let mut pending_rename_start: Option<(usize, String)> = None; // (pane_idx, current_title)
        let mut pending_rename_commit = false;
        let mut pending_rename_cancel = false;

        for pane_idx in 0..num_panes {
            let full_rect = pane_rects[pane_idx];
            let is_active = pane_idx == active_pane_idx;
            let pane_uid = self.terminal_tabs[tab_idx].panes[pane_idx].uid;
            let pane_id = ui.id().with(("pane_uid", pane_uid));

            // Split full rect into bar + content
            let bar_rect =
                egui::Rect::from_min_size(full_rect.min, egui::vec2(full_rect.width(), BAR_H));
            let content_rect = egui::Rect::from_min_max(
                egui::pos2(full_rect.min.x, full_rect.min.y + BAR_H),
                full_rect.max,
            );

            // ── Title bar ──────────────────────────────────────────────────
            let bar_bg = if is_active {
                palette.surface
            } else {
                palette.bar_bg
            };
            ui.painter()
                .rect_filled(bar_rect, egui::CornerRadius::ZERO, bar_bg);

            // Drag handle (left side) — click focuses pane, drag swaps
            let handle_rect = egui::Rect::from_min_size(
                egui::pos2(bar_rect.left(), bar_rect.top()),
                egui::vec2(28.0, BAR_H),
            );
            let handle_id = pane_id.with("bar_handle");
            let handle_resp = ui.interact(handle_rect, handle_id, egui::Sense::click_and_drag());
            let handle_color = if handle_resp.hovered() || handle_resp.dragged() {
                palette.accent
            } else {
                palette.muted_text.linear_multiply(0.5)
            };
            // Draw ⠿ grid dots as 6 tiny circles arranged 2×3
            {
                let cx = handle_rect.center().x;
                let cy = handle_rect.center().y;
                let dx = 3.0_f32;
                let dy = 3.0_f32;
                let r = 1.2_f32;
                for row in [-1i32, 0, 1] {
                    for col in [-1i32, 1] {
                        ui.painter().circle_filled(
                            egui::pos2(cx + col as f32 * dx, cy + row as f32 * dy),
                            r,
                            handle_color,
                        );
                    }
                }
            }

            if handle_resp.clicked() {
                pending_focus = Some(pane_idx);
            }
            if handle_resp.drag_started() {
                let handle_center = handle_rect.center();
                ui.data_mut(|d| {
                    d.insert_temp(egui::Id::new("bar_drag_from"), pane_idx);
                    d.insert_temp(egui::Id::new("bar_drag_origin"), handle_center);
                });
            }
            if handle_resp.drag_stopped() {
                let from: Option<usize> = ui.data(|d| d.get_temp(egui::Id::new("bar_drag_from")));
                if let Some(from_idx) = from {
                    if let Some(pos) = handle_resp.interact_pointer_pos() {
                        for (to_idx, to_rect) in pane_rects.iter().enumerate() {
                            if to_idx != from_idx && to_rect.contains(pos) {
                                pending_swap = Some((from_idx, to_idx));
                                break;
                            }
                        }
                    }
                }
                ui.data_mut(|d| {
                    d.remove_by_type::<usize>();
                    d.remove_by_type::<egui::Pos2>();
                });
            }

            // X close button (right side)
            let close_btn_size = egui::vec2(BAR_H, BAR_H);
            let close_rect = egui::Rect::from_min_size(
                egui::pos2(bar_rect.right() - close_btn_size.x, bar_rect.top()),
                close_btn_size,
            );
            let close_id = pane_id.with("bar_close");
            let close_resp = ui.interact(close_rect, close_id, egui::Sense::click());
            let close_color = if close_resp.hovered() {
                egui::Color32::from_rgb(220, 80, 80)
            } else {
                palette.muted_text.linear_multiply(0.5)
            };
            ui.painter().text(
                close_rect.center(),
                egui::Align2::CENTER_CENTER,
                "×",
                egui::FontId::proportional(14.0),
                close_color,
            );
            if close_resp.clicked() {
                pending_close = Some(pane_idx);
            }

            // Pane title (center of bar, between handle and close button)
            let title_rect = egui::Rect::from_min_max(
                egui::pos2(bar_rect.left() + 28.0, bar_rect.top()),
                egui::pos2(bar_rect.right() - close_btn_size.x, bar_rect.bottom()),
            );
            let is_renaming = self.renaming_pane == Some((tab_idx, pane_idx));

            if is_renaming {
                // Inline text edit for rename
                let rename_id = pane_id.with("bar_rename_edit");
                let mut rename_ui =
                    ui.new_child(egui::UiBuilder::new().max_rect(title_rect.shrink(2.0)));
                let edit_resp = rename_ui.add(
                    egui::TextEdit::singleline(&mut self.pane_rename_buffer)
                        .id(rename_id)
                        .desired_width(title_rect.width() - 4.0)
                        .font(egui::TextStyle::Small)
                        .frame(false),
                );
                edit_resp.request_focus();
                // Check keys via ctx — single-line TextEdit does NOT lose focus on Enter
                let pressed_enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
                let pressed_esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
                if pressed_esc {
                    pending_rename_cancel = true;
                } else if pressed_enter {
                    pending_rename_commit = true;
                } else if edit_resp.lost_focus() {
                    // Clicked somewhere else — commit
                    pending_rename_commit = true;
                }
            } else {
                // Display title; double-click to rename
                let current_title = &self.terminal_tabs[tab_idx].panes[pane_idx].title;
                let display_title = if current_title.is_empty() {
                    format!("Terminal {}", pane_idx + 1)
                } else {
                    current_title.clone()
                };
                let title_color = if is_active {
                    palette.text
                } else {
                    palette.muted_text
                };
                let title_id = pane_id.with("bar_title");
                let title_resp = ui.interact(title_rect, title_id, egui::Sense::click());
                ui.painter().text(
                    title_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &display_title,
                    egui::FontId::proportional(11.0),
                    title_color,
                );
                if title_resp.double_clicked() {
                    pending_rename_start = Some((pane_idx, display_title));
                }
                if title_resp.clicked() {
                    pending_focus = Some(pane_idx);
                }
            }

            // ── Terminal content ────────────────────────────────────────────
            let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(content_rect));
            child_ui.set_clip_rect(content_rect);

            let pane = &mut self.terminal_tabs[tab_idx].panes[pane_idx];
            Self::render_pane(pane, &mut child_ui, palette, ctx, pane_id, is_active);
            let logs =
                std::mem::take(&mut self.terminal_tabs[tab_idx].panes[pane_idx].pending_logs);
            for msg in logs {
                self.log_debug(msg);
            }

            if self.terminal_tabs[tab_idx].panes[pane_idx].has_focus && !is_active {
                let old_active = self.terminal_tabs[tab_idx].active_pane;
                self.terminal_tabs[tab_idx].active_pane = pane_idx;
                self.log_debug(format!(
                    "focus_change: pane {old_active} -> {pane_idx} (uid={})",
                    self.terminal_tabs[tab_idx].panes[pane_idx].uid
                ));
            }
        }

        // Draw drag line while a bar handle is being dragged
        {
            let from: Option<usize> = ui.data(|d| d.get_temp(egui::Id::new("bar_drag_from")));
            if from.is_some() {
                if let Some(origin_pos) =
                    ui.data(|d| d.get_temp::<egui::Pos2>(egui::Id::new("bar_drag_origin")))
                {
                    if let Some(ptr) = ctx.input(|i| i.pointer.hover_pos()) {
                        ui.painter().line_segment(
                            [origin_pos, ptr],
                            egui::Stroke::new(1.5, palette.accent.linear_multiply(0.55)),
                        );
                        ctx.request_repaint();
                    }
                }
            }
        }

        // Apply pending rename
        if let Some((pane_idx, current)) = pending_rename_start {
            self.renaming_pane = Some((tab_idx, pane_idx));
            self.pane_rename_buffer = current;
        }
        if pending_rename_commit {
            if let Some((t, p)) = self.renaming_pane {
                let new_title = self.pane_rename_buffer.trim().to_owned();
                self.terminal_tabs[t].panes[p].title = new_title;
            }
            self.renaming_pane = None;
        }
        if pending_rename_cancel {
            self.renaming_pane = None;
        }

        // Apply pending focus / swap / close
        if let Some(pane_idx) = pending_focus {
            self.terminal_tabs[tab_idx].active_pane = pane_idx;
        }
        if let Some((from, to)) = pending_swap {
            let from_uid = self.terminal_tabs[tab_idx].panes[from].uid;
            let to_uid = self.terminal_tabs[tab_idx].panes[to].uid;
            self.terminal_tabs[tab_idx].panes.swap(from, to);
            let active = self.terminal_tabs[tab_idx].active_pane;
            if active == from {
                self.terminal_tabs[tab_idx].active_pane = to;
            } else if active == to {
                self.terminal_tabs[tab_idx].active_pane = from;
            }
            self.log_debug(format!(
                "bar_swap: {from}(uid={from_uid}) <-> {to}(uid={to_uid})"
            ));
        }
        if let Some(close_idx) = pending_close {
            let before = self.terminal_tabs[tab_idx].panes.len();
            self.terminal_tabs[tab_idx].close_pane(close_idx);
            self.log_debug(format!(
                "bar_close: pane {close_idx}, {before} -> {} panes",
                self.terminal_tabs[tab_idx].panes.len()
            ));
            // Cancel rename if it was for the closed pane
            if self.renaming_pane == Some((tab_idx, close_idx)) {
                self.renaming_pane = None;
            }
        }

        // Reserve the full grid area so egui knows it's used
        let grid_rect = egui::Rect::from_min_size(
            origin,
            egui::vec2(total_width, rows as f32 * (pane_height + gap) - gap),
        );
        ui.allocate_rect(grid_rect, egui::Sense::hover());
    }

}
