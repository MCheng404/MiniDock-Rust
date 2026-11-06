//! About page: version, project link, author, license.

use egui::{Align2, Color32, FontId, Rect, RichText, Sense, Stroke, Ui, Vec2};

use super::about_art::MARK;
use super::widgets::{card, Ctx};
use super::Command;

const PROJECT_URL: &str = "https://github.com/MCheng404/MiniDock-Rust";
const AUTHOR_URL: &str = "https://github.com/MCheng404";
const LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

/// Height of a link row: two text lines plus breathing room.
const ROW_H: f32 = 44.0;
/// Square canvas the right-hand art is drawn into.
const ART: f32 = 32.0;

pub(super) fn page(ctx: &mut Ctx<'_>, ui: &mut Ui) {
    card(ctx, ui, "MiniDock", "Windows 桌面 Dock。", |_ctx, ui| {
        value_row(ui, "版本", minidock_core::VERSION);
        value_row(
            ui,
            "构建",
            if cfg!(debug_assertions) {
                "Debug"
            } else {
                "Release"
            },
        );
    });

    card(
        ctx,
        ui,
        "项目",
        "源码、发行版与问题反馈。",
        |ctx, ui| {
            link_row(ctx, ui, "项目地址", PROJECT_URL, paint_mark);
        },
    );

    card(ctx, ui, "作者", "", |ctx, ui| {
        link_row(ctx, ui, "作者主页", AUTHOR_URL, paint_avatar);
    });

    card(ctx, ui, "许可", "自由软件许可。", |ctx, ui| {
        value_row(ui, "协议", "GNU GPL v3.0 or Later");
        ui.label(
            RichText::new(
                "可自由使用、修改与再分发；分发修改后的版本需同样以 GPL 授权，且不提供担保。",
            )
            .small()
            .weak(),
        );
        ui.add_space(4.0);
        super::widgets::button(ctx, ui, "查看许可全文", Command::OpenUrl(LICENSE_URL));
    });
}

/// Label on the left, muted value on the right.
fn value_row(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(value)
                    .monospace()
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        });
    });
    ui.add_space(7.0);
}

/// A clickable row: title + url on the left, art on the right.
fn link_row(
    ctx: &mut Ctx<'_>,
    ui: &mut Ui,
    label: &str,
    url: &'static str,
    art: fn(&egui::Painter, Rect, bool),
) {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(12),
            ui.visuals().widgets.hovered.bg_fill,
        );
    }

    let dark = ui.visuals().dark_mode;
    let painter = ui.painter();
    painter.text(
        rect.left_top() + Vec2::new(0.0, 7.0),
        Align2::LEFT_TOP,
        label,
        FontId::proportional(14.0),
        ui.visuals().text_color(),
    );
    painter.text(
        rect.left_top() + Vec2::new(0.0, 25.0),
        Align2::LEFT_TOP,
        url,
        FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );

    let canvas = Rect::from_center_size(
        rect.right_center() - Vec2::new(ART * 0.5, 0.0),
        Vec2::splat(ART),
    );
    art(painter, canvas, dark);

    if resp.clicked() {
        ctx.out.command = Some(Command::OpenUrl(url));
    }
    ui.add_space(7.0);
}

/// The GitHub mark ("invertocat"). The source path is a knockout: the disc is
/// the ink and the cat is the hole, so the paper is painted first.
fn paint_mark(painter: &egui::Painter, rect: Rect, dark: bool) {
    let (ink, paper) = if dark {
        (
            Color32::from_rgb(238, 240, 245),
            Color32::from_rgb(32, 33, 40),
        )
    } else {
        (Color32::from_rgb(24, 26, 32), Color32::WHITE)
    };
    painter.circle_filled(rect.center(), rect.width() * 0.5, paper);

    let mut mesh = egui::Mesh::default();
    mesh.vertices.reserve(MARK.len());
    for [x, y] in MARK {
        mesh.colored_vertex(
            rect.min + Vec2::new(x * rect.width(), y * rect.height()),
            ink,
        );
    }
    for i in 0..(MARK.len() / 3) as u32 {
        mesh.add_triangle(i * 3, i * 3 + 1, i * 3 + 2);
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// Author avatar: a monogram badge until a real picture ships with the build.
fn paint_avatar(painter: &egui::Painter, rect: Rect, dark: bool) {
    let (bg, fg) = if dark {
        (
            Color32::from_rgb(238, 240, 245),
            Color32::from_rgb(32, 33, 40),
        )
    } else {
        (
            Color32::from_rgb(24, 26, 32),
            Color32::from_rgb(250, 251, 253),
        )
    };
    let center = rect.center();
    let r = rect.width() * 0.5;
    painter.circle_filled(center, r, bg);
    painter.circle_stroke(center, r - 0.5, Stroke::new(1.0, fg.gamma_multiply(0.25)));
    painter.text(
        center,
        Align2::CENTER_CENTER,
        "M",
        FontId::proportional(rect.width() * 0.52),
        fg,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The triangle table is generated offline; a truncated paste would show up
    /// as misaligned triangles rather than a compile error.
    #[test]
    fn mark_mesh_is_well_formed() {
        assert_eq!(MARK.len() % 3, 0, "triangle soup needs whole triangles");
        for [x, y] in MARK {
            assert!(
                (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                "vertex {x},{y} outside the unit square"
            );
        }
    }
}
