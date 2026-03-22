use eframe::egui;

use crate::theme::ThemePalette;

use super::GhostStickiesApp;

impl GhostStickiesApp {

    /// Render the tab bar
    pub(crate) fn render_tab_bar(
        &mut self,
        ui: &mut egui::Ui,
        palette: ThemePalette,
    ) -> (
        Option<usize>,
        Option<usize>,
        Option<usize>,
        Option<(usize, usize)>,
    ) {
        let mut switch_to = None;
        let mut close_tab = None;
        let mut rename_tab = None;
        let mut move_tab = None;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;

            for index in 0..self.terminal_tabs.len() {
                let selected = index == self.active_terminal;
                let renaming = self.renaming_tab == Some(index);

                if renaming {
                    let response = ui.add_sized(
                        [140.0, 28.0],
                        egui::TextEdit::singleline(&mut self.rename_buffer)
                            .clip_text(false)
                            .desired_width(140.0),
                    );

                    if response.lost_focus()
                        && ui.input(|input| input.key_pressed(egui::Key::Enter))
                    {
                        self.commit_tab_rename();
                    }

                    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                        self.cancel_tab_rename();
                    }

                    response.request_focus();
                    continue;
                }

                let (tab_fill, tab_text_color) = if selected {
                    (palette.active_tab_bg, palette.active_tab_text)
                } else {
                    (egui::Color32::TRANSPARENT, palette.tab_text)
                };

                // Show pane count if > 1
                let tab_label = {
                    let pane_count = self.terminal_tabs[index].panes.len();
                    if pane_count > 1 {
                        format!("{} ({})", self.terminal_tabs[index].title, pane_count)
                    } else {
                        self.terminal_tabs[index].title.clone()
                    }
                };

                let tab_frame = egui::Frame::NONE
                    .fill(tab_fill)
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::symmetric(12, 4));

                let response = tab_frame
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&tab_label)
                                .size(12.5)
                                .color(tab_text_color),
                        );
                    })
                    .response;

                let response = response.interact(egui::Sense::click_and_drag());

                if response.clicked() {
                    switch_to = Some(index);
                }

                if response.double_clicked() {
                    rename_tab = Some(index);
                }

                response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        rename_tab = Some(index);
                        ui.close();
                    }

                    if ui
                        .add_enabled(self.terminal_tabs.len() > 1, egui::Button::new("Close"))
                        .clicked()
                    {
                        close_tab = Some(index);
                        ui.close();
                    }
                });

                if response.dragged() {
                    if let Some(pointer_pos) = response.interact_pointer_pos() {
                        if pointer_pos.x < response.rect.left() && index > 0 {
                            move_tab = Some((index, index - 1));
                        } else if pointer_pos.x > response.rect.right()
                            && index + 1 < self.terminal_tabs.len()
                        {
                            move_tab = Some((index, index + 1));
                        }
                    }
                }
            }

            ui.add_space(4.0);
            let plus_btn = Self::tab_plus_button(ui, palette);
            if plus_btn.clicked() {
                self.add_terminal_tab();
            }
            plus_btn.on_hover_text("New tab (Cmd+T)");

            // Show split hint
            let tab = &self.terminal_tabs[self.active_terminal];
            if tab.panes.len() > 1 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "pane {}/{}",
                            tab.active_pane + 1,
                            tab.panes.len()
                        ))
                        .small()
                        .color(palette.muted_text),
                    );
                });
            }
        });

        (switch_to, close_tab, rename_tab, move_tab)
    }
}
