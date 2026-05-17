// The sticky-note frame. Paints the app window as a single paper sticky note —
// soft drop shadow, a muted-pastel paper card, an Apple-Stickies header band
// with a menu chevron + close button, and a folded dog-ear corner. Returns the
// inner rect the terminal draws into.

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::theme::ThemePalette;

const SHADOW_MARGIN: f32 = 26.0;
const HEADER: f32 = 26.0;
const DOGEAR: f32 = 18.0;
const RADIUS: f32 = 6.0;

/// The pastel paper a sticky is printed on. Muted, not neon.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub(crate) enum PaperColor {
    Lemon,
    Peach,
    Rose,
    Mint,
    Sky,
    Lilac,
    Sand,
}

impl Default for PaperColor {
    fn default() -> Self {
        Self::Lemon
    }
}

impl PaperColor {
    pub(crate) const ALL: [Self; 7] = [
        Self::Lemon,
        Self::Peach,
        Self::Rose,
        Self::Mint,
        Self::Sky,
        Self::Lilac,
        Self::Sand,
    ];

    /// Chrome colors for this paper.
    pub(crate) fn colors(self) -> StickyColors {
        // (body, header, edge highlight, dog-ear face)
        let (body, header, edge, dogear) = match self {
            Self::Lemon => ((236, 228, 168), (226, 215, 138), (245, 239, 196), (223, 214, 150)),
            Self::Peach => ((240, 214, 186), (230, 199, 167), (248, 229, 208), (228, 201, 173)),
            Self::Rose => ((238, 209, 212), (228, 192, 197), (247, 227, 229), (227, 197, 201)),
            Self::Mint => ((208, 228, 207), (186, 214, 188), (226, 240, 224), (195, 219, 196)),
            Self::Sky => ((203, 222, 234), (182, 207, 223), (224, 237, 243), (191, 212, 225)),
            Self::Lilac => ((219, 211, 233), (201, 191, 220), (236, 230, 244), (207, 199, 223)),
            Self::Sand => ((228, 220, 203), (216, 206, 184), (240, 235, 223), (218, 209, 191)),
        };
        StickyColors {
            body: rgb(body),
            header: rgb(header),
            edge_light: rgb(edge),
            dogear: rgb(dogear),
            line: egui::Color32::from_black_alpha(30),
            ink: egui::Color32::from_rgb(96, 90, 80),
            text: egui::Color32::from_rgb(54, 50, 44),
        }
    }

    /// Terminal palette derived from the paper — dark ink on the paper body.
    pub(crate) fn terminal_palette(self) -> ThemePalette {
        let c = self.colors();
        ThemePalette {
            terminal_bg: c.body,
            text: c.text,
            muted_text: c.ink,
            selection: egui::Color32::from_black_alpha(40),
            accent: c.text,
            surface: c.dogear,
        }
    }

    /// A random paper that isn't the current one.
    pub(crate) fn random_other(self) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0) as usize;
        let others: Vec<Self> = Self::ALL.into_iter().filter(|p| *p != self).collect();
        others[seed % others.len()]
    }
}

fn rgb((r, g, b): (u8, u8, u8)) -> egui::Color32 {
    egui::Color32::from_rgb(r, g, b)
}

/// Resolved chrome colors for one sticky.
#[derive(Clone, Copy)]
pub(crate) struct StickyColors {
    /// Paper body — the bulk of the note.
    pub(crate) body: egui::Color32,
    /// Header band along the top — the drag handle.
    pub(crate) header: egui::Color32,
    /// Lit top edge of the paper.
    pub(crate) edge_light: egui::Color32,
    /// Face of the folded dog-ear corner.
    pub(crate) dogear: egui::Color32,
    /// Hairline separators / fold shading.
    pub(crate) line: egui::Color32,
    /// Ink color for chrome marks (close glyph, chevron).
    pub(crate) ink: egui::Color32,
    /// Darker ink for body text.
    pub(crate) text: egui::Color32,
}

/// An action picked from the sticky's dropdown menu.
#[derive(Clone, Copy)]
pub(crate) enum MenuAction {
    /// Shuffle to a random paper color.
    Randomize,
    /// Toggle hidden-from-screen-share.
    ToggleVisibility,
    /// Collapse the note to just its header / expand it again.
    ToggleMinimize,
}

pub(crate) struct StickyFrame {
    /// Inner rect for the sticky's content (empty when minimized).
    pub(crate) content: egui::Rect,
    /// Header band — drag here to move the sticky.
    pub(crate) drag: egui::Rect,
    pub(crate) close_clicked: bool,
    /// True while the dog-ear corner is being dragged — begin a window resize.
    pub(crate) peel_resize: bool,
    /// The menu chevron was clicked this frame.
    pub(crate) menu_clicked: bool,
    /// Rect of the menu chevron — anchor for the dropdown.
    pub(crate) menu_anchor: egui::Rect,
}

/// Paint the sticky chrome into `window` and return the content layout.
pub(crate) fn paint(
    ui: &mut egui::Ui,
    window: egui::Rect,
    c: StickyColors,
    minimized: bool,
) -> StickyFrame {
    let painter = ui.painter().clone();

    // Card sits inside the window with room for the shadow.
    let card = egui::Rect::from_min_max(
        window.min + egui::vec2(SHADOW_MARGIN * 0.5, SHADOW_MARGIN * 0.4),
        window.max - egui::vec2(SHADOW_MARGIN, SHADOW_MARGIN),
    );
    let radius = egui::CornerRadius::same(RADIUS as u8);

    // ── Soft drop shadow — one blurred rect, offset down. ──
    let shadow = egui::epaint::Shadow {
        offset: [0, 9],
        blur: 24,
        spread: 0,
        color: egui::Color32::from_black_alpha(66),
    };
    painter.add(shadow.as_shape(card, radius));

    // ── Paper body ──
    painter.rect_filled(card, radius, c.body);

    // ── Header band — a slightly deeper tint across the top. ──
    let header_rect =
        egui::Rect::from_min_max(card.min, egui::pos2(card.max.x, card.min.y + HEADER));
    painter.rect_filled(
        header_rect,
        egui::CornerRadius {
            nw: RADIUS as u8,
            ne: RADIUS as u8,
            sw: 0,
            se: 0,
        },
        c.header,
    );
    painter.line_segment(
        [
            egui::pos2(card.min.x, header_rect.max.y),
            egui::pos2(card.max.x, header_rect.max.y),
        ],
        egui::Stroke::new(1.0, c.line),
    );
    painter.line_segment(
        [
            egui::pos2(card.min.x + RADIUS, card.min.y + 0.6),
            egui::pos2(card.max.x - RADIUS, card.min.y + 0.6),
        ],
        egui::Stroke::new(1.2, c.edge_light),
    );

    // A small ">_" mark on the header left — identifies the terminal note.
    painter.text(
        egui::pos2(card.min.x + 12.0, header_rect.center().y),
        egui::Align2::LEFT_CENTER,
        ">_",
        egui::FontId::monospace(11.0),
        c.ink,
    );

    // ── Dog-ear (bottom-right) — folded corner + resize grip (hidden when minimized) ──
    let mut peel_resize = false;
    if !minimized {
        let br = card.max;
        let ear_rect = egui::Rect::from_min_max(egui::pos2(br.x - DOGEAR, br.y - DOGEAR), br);
        let ear_resp = ui.interact(
            ear_rect,
            ui.id().with("sticky_dogear"),
            egui::Sense::click_and_drag(),
        );
        if ear_resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
        }
        let fold_a = egui::pos2(br.x - DOGEAR, br.y);
        let fold_c = egui::pos2(br.x, br.y - DOGEAR);
        // Soft shadow the lifted flap casts onto the page.
        painter.add(egui::Shape::convex_polygon(
            vec![
                fold_a,
                fold_c,
                egui::pos2(br.x - DOGEAR * 0.3, br.y - DOGEAR * 0.3),
            ],
            egui::Color32::from_black_alpha(44),
            egui::Stroke::NONE,
        ));
        // The folded flap itself.
        painter.add(egui::Shape::convex_polygon(
            vec![fold_a, fold_c, br],
            c.dogear,
            egui::Stroke::new(1.0, c.line),
        ));
        peel_resize = ear_resp.drag_started();
    }

    let header_mid = header_rect.center().y;

    // ── Close button — × at the header's right. ──
    let close_rect = egui::Rect::from_min_size(
        egui::pos2(card.max.x - 24.0, header_mid - 8.5),
        egui::vec2(17.0, 17.0),
    );
    let close_resp = ui.interact(close_rect, ui.id().with("sticky_close"), egui::Sense::click());
    let x_color = if close_resp.hovered() {
        egui::Color32::from_rgb(220, 86, 74)
    } else {
        c.ink
    };
    let cc = close_rect.center();
    let s = 3.6;
    let x_stroke = egui::Stroke::new(1.8, x_color);
    painter.line_segment([cc - egui::vec2(s, s), cc + egui::vec2(s, s)], x_stroke);
    painter.line_segment([cc + egui::vec2(s, -s), cc + egui::vec2(-s, s)], x_stroke);

    // ── Menu chevron — ▾ just left of the close button. ──
    let menu_rect = egui::Rect::from_min_size(
        egui::pos2(card.max.x - 47.0, header_mid - 8.5),
        egui::vec2(17.0, 17.0),
    );
    let menu_resp = ui.interact(menu_rect, ui.id().with("sticky_menu"), egui::Sense::click());
    let chev_color = if menu_resp.hovered() {
        c.text
    } else {
        c.ink
    };
    let mc = menu_rect.center();
    let chev = egui::Stroke::new(1.8, chev_color);
    painter.line_segment(
        [egui::pos2(mc.x - 4.0, mc.y - 2.0), egui::pos2(mc.x, mc.y + 2.5)],
        chev,
    );
    painter.line_segment(
        [egui::pos2(mc.x, mc.y + 2.5), egui::pos2(mc.x + 4.0, mc.y - 2.0)],
        chev,
    );

    let drag = egui::Rect::from_min_max(
        card.min,
        egui::pos2(card.max.x - 52.0, card.min.y + HEADER),
    );
    let content = if minimized {
        egui::Rect::from_min_max(card.min, card.min)
    } else {
        egui::Rect::from_min_max(
            egui::pos2(card.min.x + 10.0, card.min.y + HEADER + 6.0),
            egui::pos2(card.max.x - 10.0, card.max.y - 10.0),
        )
    };

    StickyFrame {
        content,
        drag,
        close_clicked: close_resp.clicked(),
        peel_resize,
        menu_clicked: menu_resp.clicked(),
        menu_anchor: menu_rect,
    }
}

/// Paint the dropdown menu under `anchor`. Returns the action clicked this
/// frame (if any) and the menu's rect (for outside-click dismissal).
pub(crate) fn paint_menu(
    ui: &mut egui::Ui,
    anchor: egui::Rect,
    c: StickyColors,
    privacy: bool,
    minimized: bool,
) -> (Option<MenuAction>, egui::Rect) {
    let painter = ui.painter().clone();

    const W: f32 = 198.0;
    const ROW_H: f32 = 31.0;
    const PAD: f32 = 6.0;
    let menu = egui::Rect::from_min_size(
        egui::pos2(anchor.right() - W, anchor.bottom() + 7.0),
        egui::vec2(W, PAD * 2.0 + ROW_H * 3.0),
    );
    let m_radius = egui::CornerRadius::same(9);

    let shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: egui::Color32::from_black_alpha(80),
    };
    painter.add(shadow.as_shape(menu, m_radius));
    painter.rect(
        menu,
        m_radius,
        c.body,
        egui::Stroke::new(1.0, c.line),
        egui::StrokeKind::Inside,
    );

    let items = [
        (MenuAction::Randomize, "Shuffle colour", false, false),
        (MenuAction::ToggleVisibility, "Hide when sharing", true, privacy),
        (
            MenuAction::ToggleMinimize,
            if minimized { "Expand note" } else { "Minimise note" },
            false,
            false,
        ),
    ];

    let mut clicked = None;
    for (i, (action, label, has_dot, dot_on)) in items.iter().enumerate() {
        let row = egui::Rect::from_min_size(
            egui::pos2(menu.left() + PAD, menu.top() + PAD + ROW_H * i as f32),
            egui::vec2(W - PAD * 2.0, ROW_H),
        );
        let resp = ui.interact(row, ui.id().with(("sticky_menu_item", i)), egui::Sense::click());
        if resp.hovered() {
            painter.rect_filled(row, egui::CornerRadius::same(6), c.header);
        }
        painter.text(
            egui::pos2(row.left() + 12.0, row.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            c.text,
        );
        if *has_dot {
            let center = egui::pos2(row.right() - 14.0, row.center().y);
            if *dot_on {
                painter.circle_filled(center, 4.5, egui::Color32::from_rgb(86, 168, 92));
            } else {
                painter.circle_stroke(center, 4.0, egui::Stroke::new(1.3, c.ink));
            }
        }
        if resp.clicked() {
            clicked = Some(*action);
        }
    }

    (clicked, menu)
}
