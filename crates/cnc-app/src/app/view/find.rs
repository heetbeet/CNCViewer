use super::*;
use crate::app::find::Results;
use egui::{epaint::Shape, text::CharIndex, text_edit::TextEditOutput, Painter};

const FIND_ROW: f32 = 28.;
const FIND_BUTTON: f32 = 23.;
const FIND_OPTIONS: f32 = 75.;
const FIND_NAV: f32 = 137.;

impl App {
    pub(super) fn find_bar(&mut self, ui: &mut egui::Ui, code: Rect) -> f32 {
        self.refresh_find(ui.ctx());
        let width = code.width().min(440.);
        let narrow = width < 325.;
        let rows = 1 + u8::from(narrow) + u8::from(self.find.replace_open);
        let height = f32::from(rows) * FIND_ROW + 6.;
        let rect = Rect::from_min_size(
            Pos2::new(code.max.x - width, code.min.y),
            Vec2::new(width, height),
        );
        let mut bar = child(ui, rect, "find-bar");
        bar.spacing_mut().button_padding.x = 3.;
        if self.settings.dark {
            bar.style_mut().visuals = egui::Visuals::dark();
        }
        let fill = if self.settings.dark {
            Color32::from_gray(46)
        } else {
            Color32::from_gray(245)
        };
        bar.painter().rect_filled(rect, 0., fill);
        bar.painter().line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            bar.visuals().widgets.noninteractive.bg_stroke,
        );
        let row = Rect::from_min_size(
            rect.min + Vec2::new(3., 3.),
            Vec2::new(width - 6., FIND_ROW),
        );
        let mut enter = false;
        let mut backwards = false;
        let id = self.find_id();
        if bar.memory(|memory| memory.has_focus(id)) {
            backwards = bar.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Enter));
            enter = backwards || bar.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
        }
        let nav_y = row.min.y + if narrow { FIND_ROW } else { 0. };
        let options_x = row.max.x - FIND_OPTIONS - if narrow { 0. } else { FIND_NAV };
        let disclosure = cell(row.min.x, row.min.y, FIND_BUTTON);
        if button_response(&mut bar, disclosure, "toggle-replace", "Toggle Replace").clicked() {
            self.find.replace_open = !self.find.replace_open;
        }
        let arrow = if self.find.replace_open {
            [
                Vec2::new(-3., -1.5),
                Vec2::new(0., 1.5),
                Vec2::new(3., -1.5),
            ]
        } else {
            [
                Vec2::new(-1.5, -3.),
                Vec2::new(1.5, 0.),
                Vec2::new(-1.5, 3.),
            ]
        };
        bar.painter().add(Shape::line(
            arrow.map(|point| disclosure.center() + point).to_vec(),
            Stroke::new(1.3, bar.visuals().text_color()),
        ));
        let input_x = row.min.x + FIND_BUTTON;
        let input = cell(input_x, row.min.y, options_x - input_x - 3.);
        let mut changed = search_input(&mut bar, input, id, &mut self.find.query, "Find");
        for (i, (value, caption, tip)) in [
            (&mut self.find.case_sensitive, "Aa", "Match Case"),
            (&mut self.find.whole_word, "Ab", "Match Whole Word"),
            (&mut self.find.regex, ".*", "Use Regular Expression"),
        ]
        .into_iter()
        .enumerate()
        {
            if find_button(
                &mut bar,
                cell(options_x + i as f32 * 25., row.min.y, 25.),
                caption,
                tip,
                true,
                *value,
            ) {
                *value = !*value;
                changed = true;
                bar.memory_mut(|memory| memory.request_focus(id));
            }
        }
        if changed {
            self.find.results = None;
            self.refresh_find(ui.ctx());
            self.find.reveal = true;
        }

        let nav_x = row.max.x - FIND_NAV;
        let results = self.find.results.as_ref();
        let found = results.is_some_and(|r| !r.ranges.is_empty());
        let previous = find_button(
            &mut bar,
            cell(nav_x + FIND_NAV - 3. * FIND_BUTTON, nav_y, FIND_BUTTON),
            "↑",
            "Previous Match (Shift+F3)",
            found,
            false,
        );
        let next = find_button(
            &mut bar,
            cell(nav_x + FIND_NAV - 2. * FIND_BUTTON, nav_y, FIND_BUTTON),
            "↓",
            "Next Match (F3)",
            found,
            false,
        );
        if previous || next || enter {
            self.find.advance(previous || backwards);
            bar.memory_mut(|memory| memory.request_focus(id));
        }
        let count_rect = cell(nav_x + 3., nav_y, FIND_NAV - 3. * FIND_BUTTON - 5.);
        if let Some(results) = &self.find.results {
            let caption = if results.error.is_some() {
                "Invalid regex".into()
            } else if found {
                format!("{} of {}", results.active + 1, results.ranges.len())
            } else if self.find.query.is_empty() {
                String::new()
            } else {
                "No results".into()
            };
            if !caption.is_empty() {
                let text = egui::RichText::new(caption).size(11.5);
                let response = bar.put(
                    count_rect,
                    egui::Label::new(if results.error.is_some() {
                        text.color(bar.visuals().error_fg_color)
                    } else {
                        text
                    })
                    .truncate(),
                );
                if let Some(error) = &results.error {
                    response.on_hover_text(error);
                }
            }
        }
        if find_button(
            &mut bar,
            cell(row.max.x - FIND_BUTTON, nav_y, FIND_BUTTON),
            "×",
            "Close Find (Esc)",
            true,
            false,
        ) {
            self.close_find(ui.ctx());
        }

        if rows > 1 + u8::from(narrow) {
            let y = nav_y + FIND_ROW;
            let id = self.local_id("editor-replace");
            let replace = bar.memory(|memory| memory.has_focus(id))
                && bar.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
            search_input(
                &mut bar,
                cell(input_x, y, row.max.x - input_x - 102.),
                id,
                &mut self.find.replacement,
                "Replace",
            );
            let replace_one = find_button(
                &mut bar,
                cell(row.max.x - 98., y, 63.),
                "Replace",
                "Replace Current Match",
                found,
                false,
            );
            let replace_all = find_button(
                &mut bar,
                cell(row.max.x - 34., y, 34.),
                "All",
                "Replace All Matches",
                found,
                false,
            );
            if replace_all || replace_one || replace {
                self.replace_matches(ui.ctx(), replace_all);
                bar.memory_mut(|memory| memory.request_focus(id));
            }
        }
        height
    }
}

fn cell(x: f32, y: f32, width: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y + 2.), Vec2::new(width, FIND_ROW - 4.))
}

fn search_input(
    ui: &mut egui::Ui,
    rect: Rect,
    id: egui::Id,
    value: &mut String,
    hint: &str,
) -> bool {
    ui.put(
        rect,
        egui::TextEdit::singleline(value)
            .id(id)
            .hint_text(hint)
            .desired_width(rect.width())
            .min_size(rect.size()),
    )
    .changed()
}

fn find_button(
    ui: &mut egui::Ui,
    rect: Rect,
    caption: &str,
    tip: &str,
    enabled: bool,
    selected: bool,
) -> bool {
    ui.add_enabled_ui(enabled, |ui| {
        ui.put(
            rect,
            egui::Button::new(caption)
                .min_size(rect.size())
                .wrap_mode(egui::TextWrapMode::Extend)
                .selected(selected)
                .frame_when_inactive(selected),
        )
        .on_hover_text(tip)
        .clicked()
    })
    .inner
}

pub(super) fn match_highlights(
    results: &Results,
    text: &str,
    layout: CodeLayout<'_>,
    painter: &Painter,
    dark: bool,
) -> Shape {
    let clip = painter.clip_rect();
    let origin = layout.origin;
    let rows = &layout.galley.rows;
    let first = rows.partition_point(|row| origin.y + row.max_y() < clip.min.y);
    let mut highlights = Vec::new();
    for (index, row) in rows.iter().enumerate().skip(first) {
        let index = layout.first_row + index;
        if origin.y + row.min_y() > clip.max.y {
            break;
        }
        let Some(&start) = results.line_starts.get(index) else {
            break;
        };
        let next = results.line_starts.get(index + 1).copied();
        let end = next.map_or(text.len(), |byte| byte - 1);
        let first_match = results.ranges.partition_point(|range| {
            range.end < start || (range.end == start && range.start < start)
        });
        let mut byte = start;
        let mut column = 0;
        let mut x_at = |target: usize| {
            column += text[byte..target].chars().count();
            byte = target;
            origin.x + row.pos.x + row.x_offset(CharIndex(column))
        };
        for (hit, range) in results.ranges.iter().enumerate().skip(first_match) {
            if next.is_some_and(|next| range.start >= next) {
                break;
            }
            let left = x_at(range.start.clamp(start, end));
            if left > clip.max.x {
                break;
            }
            let right = x_at(range.end.min(end)).max(left + 2.);
            if right >= clip.min.x {
                let fill = match (dark, hit == results.active) {
                    (false, false) => Color32::from_rgb(255, 225, 154),
                    (false, true) => Color32::from_rgb(255, 176, 93),
                    (true, false) => Color32::from_rgb(86, 71, 39),
                    (true, true) => Color32::from_rgb(126, 80, 25),
                };
                let rect = Rect::from_min_max(
                    Pos2::new(left, origin.y + row.min_y()),
                    Pos2::new(right, origin.y + row.max_y()),
                );
                highlights.push(Shape::rect_filled(rect, 1., fill));
            }
        }
    }
    Shape::Vec(highlights)
}

pub(super) fn reveal_match(results: &Results, text: &str, output: &TextEditOutput, ui: &egui::Ui) {
    if let Some(range) = results.current() {
        let index = results.row_at(range.start);
        if let Some(row) = output.galley.rows.get(index) {
            let start = results.line_starts[index];
            let column = text[start..range.start].chars().count();
            let position = output.galley_pos + row.pos.to_vec2();
            let rect = Rect::from_min_size(
                position + Vec2::new(row.x_offset(CharIndex(column)), 0.),
                Vec2::new(5., row.size.y),
            );
            ui.scroll_to_rect_animation(
                rect.expand(5.),
                None,
                egui::style::ScrollAnimation::none(),
            );
        }
    }
}
