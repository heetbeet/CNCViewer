use super::*;
use cnc_core::error::UNASSIGNED_VARIABLE;
use egui::{layers::ShapeIdx, text_edit::TextEditOutput, Painter, Shape};

pub(super) fn for_source<'a>(diagnostics: &'a [Diagnostic], source: &str) -> &'a [Diagnostic] {
    let start = diagnostics.partition_point(|note| note.source.as_str() < source);
    let count = diagnostics[start..].partition_point(|note| note.source == source);
    &diagnostics[start..start + count]
}

pub(super) fn paint(
    ui: &mut egui::Ui,
    painter: &Painter,
    background: ShapeIdx,
    output: &TextEditOutput,
    notes: &[Diagnostic],
    size: f32,
    dark: bool,
) {
    let clip = painter.clip_rect();
    let origin = output.galley_pos;
    let rows = &output.galley.rows;
    let first = rows.partition_point(|row| origin.y + row.max_y() <= clip.min.y);
    let last = rows.partition_point(|row| origin.y + row.min_y() < clip.max.y);
    let mut notes = &notes[notes.partition_point(|note| note.line <= first)..];
    let mut bands = Vec::new();
    while let Some(note) = notes.first().filter(|note| note.line <= last) {
        let line = note.line;
        let count = notes.partition_point(|note| note.line == line);
        let line_notes = &notes[..count];
        let warning = line_notes
            .iter()
            .all(|note| note.error.code == UNASSIGNED_VARIABLE);
        let (fill, ink) = match (dark, warning) {
            (false, false) => (
                Color32::from_rgb(255, 236, 240),
                Color32::from_rgb(172, 32, 53),
            ),
            (true, false) => (
                Color32::from_rgb(75, 43, 48),
                Color32::from_rgb(255, 136, 149),
            ),
            (false, true) => (
                Color32::from_rgb(255, 245, 220),
                Color32::from_rgb(128, 76, 9),
            ),
            (true, true) => (
                Color32::from_rgb(67, 54, 32),
                Color32::from_rgb(244, 190, 106),
            ),
        };
        let mut message = line_notes
            .iter()
            .map(|note| note.error.message.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        notes = &notes[count..];
        if message.contains(['\r', '\n']) {
            message = message.replace(['\r', '\n'], " ");
        }
        let row = rows[line - 1].rect().translate(origin.to_vec2());
        let band = Rect::from_min_max(
            Pos2::new(clip.min.x, row.min.y),
            Pos2::new(clip.max.x, row.max.y),
        )
        .intersect(clip);
        bands.push(Shape::rect_filled(band, 0., fill));
        let galley = painter.layout_no_wrap(format!("·  {message}"), FontId::monospace(size), ink);
        let position = Pos2::new(row.max.x + 10., row.center().y - galley.size().y / 2.);
        if position.x < clip.min.x || position.x + galley.size().x > clip.max.x {
            ui.interact(
                band,
                ui.scope_id().with(("inline-diagnostic", line)),
                Sense::hover(),
            )
            .on_hover_ui_at_pointer(|ui| {
                ui.label(&message);
            });
        }
        painter.galley(position, galley, ink);
    }
    painter.set(background, Shape::Vec(bands));
}
