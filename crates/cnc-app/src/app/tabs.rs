use super::{history::History, session::Session};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke};
use std::{
    borrow::Cow,
    path::{Path, PathBuf},
};

pub(super) const HEIGHT: f32 = 25.;
const HISTORY_ROW_HEIGHT: f32 = 22.;
const HISTORY_SCROLL_HEIGHT: f32 = 330.;
const HISTORY_MAX_WIDTH: f32 = 540.;
const HISTORY_NAME_SIZE: f32 = 12.;
const HISTORY_DATE_SIZE: f32 = 11.;
const HISTORY_LEFT_PAD: f32 = 6.;
const HISTORY_RIGHT_PAD: f32 = 10.;
const HISTORY_COLUMN_GAP: f32 = 12.;

pub(super) struct Label {
    pub text: String,
    pub path: Option<PathBuf>,
    pub dirty: bool,
}

#[derive(Debug)]
pub(super) enum Command {
    Select(usize),
    New,
    Sample,
    Open(PathBuf),
    Close(usize),
    Drag(usize),
    Split(usize),
    Detach(usize),
}

pub(super) struct Strip<'a> {
    pub labels: &'a [Label],
    pub selected: usize,
    pub history: &'a mut History,
    pub session: Option<&'a Session>,
    pub reveal: bool,
    pub commands: Vec<Command>,
    pub rect: Option<Rect>,
    pub hits: Vec<(usize, Rect)>,
}

impl Strip<'_> {
    pub fn show(&mut self, ui: &mut egui::Ui, rect: Rect) {
        self.rect = Some(rect);
        let mut bar = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("tabs")
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        bar.set_clip_rect(rect.intersect(ui.clip_rect()));
        bar.style_mut().always_scroll_the_only_direction = true;
        bar.painter().rect_filled(rect, 0., Color32::from_gray(239));
        let bottom = rect.bottom() - 0.5;
        bar.painter().hline(
            rect.x_range(),
            bottom,
            Stroke::new(1., Color32::from_gray(217)),
        );
        egui::ScrollArea::horizontal()
            .id_salt("open-tabs")
            .auto_shrink([false, false])
            .scroll_source(
                egui::scroll_area::ScrollSource::SCROLL_BAR
                    | egui::scroll_area::ScrollSource::MOUSE_WHEEL,
            )
            .show(&mut bar, |ui| {
                ui.spacing_mut().item_spacing.x = 1.;
                ui.horizontal(|ui| {
                    let start = ui.cursor().min.x;
                    for (index, label) in self.labels.iter().enumerate() {
                        let ink = Color32::from_gray(65);
                        let font = FontId::proportional(13.);
                        let caption =
                            ui.painter()
                                .layout_no_wrap(label.text.clone(), font.clone(), ink);
                        let width = (caption.size().x + 40.).clamp(80., 225.);
                        let (rect, response) = ui
                            .allocate_exact_size(vec2(width, HEIGHT - 2.), Sense::click_and_drag());
                        let selected = index == self.selected;
                        if selected && self.reveal {
                            response.scroll_to_me(None);
                        }
                        self.hits.push((index, rect.intersect(ui.clip_rect())));
                        let fill = if selected {
                            Color32::WHITE
                        } else if response.hovered() {
                            Color32::from_gray(245)
                        } else {
                            Color32::from_gray(228)
                        };
                        let painter = ui.painter();
                        painter.rect_filled(rect, 2., fill);
                        if selected {
                            painter.rect_filled(
                                Rect::from_min_size(rect.min, vec2(width, 2.)),
                                0.,
                                Color32::from_rgb(104, 182, 136),
                            );
                        }
                        if label.dirty {
                            painter.circle_filled(
                                pos2(rect.left() + 9., rect.center().y),
                                3.,
                                Color32::from_rgb(51, 131, 91),
                            );
                        }
                        let close = Rect::from_center_size(
                            pos2(rect.right() - 11., rect.center().y),
                            vec2(18., 22.),
                        );
                        let caption = if caption.size().x > width - 38. {
                            egui::WidgetText::from(label.text.as_str()).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                width - 38.,
                                font,
                            )
                        } else {
                            caption
                        };
                        painter.galley(
                            pos2(rect.left() + 16., rect.center().y - caption.size().y / 2.),
                            caption,
                            ink,
                        );
                        painter.text(
                            close.center(),
                            Align2::CENTER_CENTER,
                            "×",
                            FontId::proportional(14.),
                            ink,
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::Button, true, &label.text)
                        });
                        if response.middle_clicked()
                            || response.clicked()
                                && response
                                    .interact_pointer_pos()
                                    .is_some_and(|p| close.contains(p))
                        {
                            self.commands.push(Command::Close(index));
                        } else if response.clicked() {
                            self.commands.push(Command::Select(index));
                        }
                        if response.drag_started_by(egui::PointerButton::Primary) {
                            self.commands.push(Command::Drag(index));
                        }
                        let response = if let Some(path) = &label.path {
                            response.on_hover_text(path.display().to_string())
                        } else {
                            response
                        };
                        response.context_menu(|ui| {
                            for (caption, action) in [
                                ("Move to Other Pane", Command::Split(index)),
                                ("Move to New Window", Command::Detach(index)),
                                ("Close Tab", Command::Close(index)),
                            ] {
                                if ui.button(caption).clicked() {
                                    self.commands.push(action);
                                    ui.close();
                                }
                            }
                            if ui
                                .add_enabled(
                                    label.path.is_some(),
                                    egui::Button::new("Copy Filepath"),
                                )
                                .clicked()
                            {
                                if let Some(path) = &label.path {
                                    ui.ctx().copy_text(path.display().to_string());
                                    ui.close();
                                }
                            }
                        });
                    }
                    let remaining = rect.width() - (ui.cursor().min.x - start);
                    let (_, response) = ui
                        .allocate_exact_size(vec2(remaining.max(30.), HEIGHT - 2.), Sense::click());
                    if response.double_clicked() {
                        self.commands.push(Command::New);
                    }
                });
            });
    }

    pub fn history_menu(&mut self, response: &egui::Response) {
        let popup = egui::Popup::menu(response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let opening = response.clicked() && !popup.is_open();
        if opening {
            self.history.open(self.session, &response.ctx);
        } else if !popup.is_open() {
            return;
        }
        let history = &self.history;
        let ctx = &response.ctx;
        let layout_id = response.id.with("history-layout");
        let (name_start, preferred_width) = match ctx.data(|data| data.get_temp(layout_id)) {
            Some(layout) if !opening => layout,
            _ => {
                let layout = history_layout(ctx, history);
                ctx.data_mut(|data| data.insert_temp(layout_id, layout));
                layout
            }
        };
        let width =
            preferred_width.min(ctx.input(|input| (input.content_rect().width() - 24.).max(1.)));
        popup.width(width).show(|ui| {
            ui.set_width(width);
            if history_row(ui, name_start, None, "Sample", true).clicked() {
                self.commands.push(Command::Sample);
                ui.close();
            }
            if history.rows.is_empty() {
                return;
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(HISTORY_SCROLL_HEIGHT)
                .auto_shrink([false, true])
                .show_rows(ui, HISTORY_ROW_HEIGHT, history.rows.len(), |ui, range| {
                    for recent in &history.rows[range] {
                        let path = &recent.path;
                        let date = chrono::DateTime::from_timestamp_micros(recent.opened as i64)
                            .map(|time| {
                                time.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d")
                                    .to_string()
                            })
                            .unwrap_or_default();
                        let name = history_name(path);
                        let available = !history.missing.contains(recent);
                        let row = history_row(ui, name_start, Some(&date), &name, available);
                        if row.on_hover_text(path.display().to_string()).clicked() {
                            self.commands.push(Command::Open(path.clone()));
                            ui.close();
                        }
                    }
                });
        });
    }
}

fn history_name(path: &Path) -> Cow<'_, str> {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
}

fn history_layout(ctx: &egui::Context, history: &History) -> (f32, f32) {
    let style = ctx.global_style();
    let spacing = &style.spacing;
    let scroll_padding = if history.rows.len() as f32
        * (HISTORY_ROW_HEIGHT + spacing.item_spacing.y)
        > HISTORY_SCROLL_HEIGHT
    {
        let scroll = &spacing.scroll;
        scroll.bar_width + scroll.bar_inner_margin + scroll.bar_outer_margin
    } else {
        0.
    };
    ctx.fonts_mut(|fonts| {
        let date_width = fonts
            .layout_no_wrap(
                "2000-00-00".into(),
                FontId::monospace(HISTORY_DATE_SIZE),
                Color32::PLACEHOLDER,
            )
            .size()
            .x;
        let name_start = HISTORY_LEFT_PAD + date_width + HISTORY_COLUMN_GAP;
        let margin = name_start + HISTORY_RIGHT_PAD + scroll_padding;
        let mut width = margin;
        for name in std::iter::once(Cow::Borrowed("Sample"))
            .chain(history.rows.iter().map(|entry| history_name(&entry.path)))
        {
            let text_width = fonts
                .layout_no_wrap(
                    name.into_owned(),
                    FontId::proportional(HISTORY_NAME_SIZE),
                    Color32::PLACEHOLDER,
                )
                .size()
                .x;
            width = width.max(margin + text_width);
            if width >= HISTORY_MAX_WIDTH {
                break;
            }
        }
        (name_start, width.ceil().min(HISTORY_MAX_WIDTH))
    })
}

fn history_row(
    ui: &mut egui::Ui,
    name_start: f32,
    date: Option<&str>,
    name: &str,
    available: bool,
) -> egui::Response {
    let sense = if available {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), HISTORY_ROW_HEIGHT), sense);
    row.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            available,
            format!("{} {name}", date.unwrap_or_default()).trim(),
        )
    });
    let visuals = ui.style().interact(&row);
    let painter = ui.painter();
    let color = if available {
        if row.hovered() || row.has_focus() {
            painter.rect_filled(rect, 2., visuals.weak_bg_fill);
        }
        visuals.text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    if let Some(date) = date {
        painter.text(
            rect.left_center() + vec2(HISTORY_LEFT_PAD, 0.),
            Align2::LEFT_CENTER,
            date,
            FontId::monospace(HISTORY_DATE_SIZE),
            color,
        );
    } else {
        let center = rect.left_center()
            + vec2(
                (HISTORY_LEFT_PAD + name_start - HISTORY_COLUMN_GAP) / 2.,
                0.,
            );
        let stroke = Stroke::new(1., color);
        painter.add(egui::Shape::closed_line(
            [(-4., -5.), (1., -5.), (4., -2.), (4., 5.), (-4., 5.)]
                .map(|(x, y)| center + vec2(x, y))
                .to_vec(),
            stroke,
        ));
        painter.add(egui::Shape::line(
            vec![
                center + vec2(1., -5.),
                center + vec2(1., -2.),
                center + vec2(4., -2.),
            ],
            stroke,
        ));
        painter.line_segment([center + vec2(-2., 1.), center + vec2(2., 1.)], stroke);
    }
    let name_x = rect.left() + name_start;
    let name = egui::WidgetText::from(name).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.right() - name_x - HISTORY_RIGHT_PAD).max(1.),
        FontId::proportional(HISTORY_NAME_SIZE),
    );
    painter.galley(
        pos2(name_x, rect.center().y - name.size().y / 2.),
        name,
        color,
    );
    row
}
