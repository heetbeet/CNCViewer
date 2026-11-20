use super::{history::History, session::Session};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, Stroke};
use std::path::PathBuf;

pub(super) const HEIGHT: f32 = 25.;

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
        if response.clicked() && !popup.is_open() {
            self.history.open(self.session, &response.ctx);
        }
        const ROW_HEIGHT: f32 = 24.;
        let width = response
            .ctx
            .input(|input| (input.content_rect().width() - 24.).clamp(200., 540.));
        let history = &self.history;
        popup.width(width).show(|ui| {
            ui.set_width(width);
            if history_row(ui, ROW_HEIGHT, "", "Sample", true).clicked() {
                self.commands.push(Command::Sample);
                ui.close();
            }
            if history.rows.is_empty() {
                return;
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(330.)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_HEIGHT, history.rows.len(), |ui, range| {
                    for recent in &history.rows[range] {
                        let path = &recent.path;
                        let date = chrono::DateTime::from_timestamp_micros(recent.opened as i64)
                            .map(|time| {
                                time.with_timezone(&chrono::Local)
                                    .format("%Y-%m-%d")
                                    .to_string()
                            })
                            .unwrap_or_default();
                        let name = path
                            .file_name()
                            .unwrap_or(path.as_os_str())
                            .to_string_lossy();
                        let available = !history.missing.contains(recent);
                        let row = history_row(ui, ROW_HEIGHT, &date, &name, available);
                        if row.on_hover_text(path.display().to_string()).clicked() {
                            self.commands.push(Command::Open(path.clone()));
                            ui.close();
                        }
                    }
                });
        });
    }
}

fn history_row(
    ui: &mut egui::Ui,
    height: f32,
    date: &str,
    name: &str,
    available: bool,
) -> egui::Response {
    let sense = if available {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), height), sense);
    row.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            available,
            format!("{date} {name}").trim(),
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
    if !date.is_empty() {
        painter.text(
            rect.left_center() + vec2(6., 0.),
            Align2::LEFT_CENTER,
            date,
            FontId::monospace(12.),
            color,
        );
    }
    let name_x = rect.left() + 100.;
    let name = egui::WidgetText::from(name).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (rect.right() - name_x - 6.).max(1.),
        FontId::proportional(13.),
    );
    painter.galley(
        pos2(name_x, rect.center().y - name.size().y / 2.),
        name,
        color,
    );
    row
}
