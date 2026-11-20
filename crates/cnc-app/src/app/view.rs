mod credits;
mod cube;
mod diagnostics;
mod find;

use super::*;
use egui::{Align2, FontId, Pos2, Rect, Stroke, UiBuilder};

pub(super) struct Highlight {
    text: String,
    size: u32,
    dpi: u32,
    dark: bool,
    rows: Option<(usize, usize)>,
    galley: Arc<egui::Galley>,
}
pub(super) const TOOLBAR_HEIGHT: f32 = 36.;
const SIDEBAR_GUTTER: f32 = 28.;
pub(super) const MIN_SIDEBAR_WIDTH: f32 = 240. + SIDEBAR_GUTTER;
const SECTION_HEIGHT: f32 = 36.;
const SECTION_TITLE_SIZE: f32 = 15.;
const SOURCE_PICKER_WIDTH: f32 = 24.;
const INK: Color32 = Color32::from_rgb(85, 85, 85);
const TOOL_INK: Color32 = Color32::from_gray(51);
const MINT: Color32 = Color32::from_rgb(143, 200, 160);
const BANNER_GREEN: Color32 = Color32::from_rgb(104, 182, 136);
const PLOT_COLORS: ([u8; 3], [u8; 3]) = ([86, 156, 110], [124, 190, 146]);
const GRAY_ACTION_COLORS: ([u8; 3], [u8; 3]) = ([55, 58, 60], [127, 130, 139]);
const DISABLED_ACTION_TEXT: Color32 = Color32::from_gray(125);
const SYNC_WIDTHS: [f32; 3] = [67., 82., 70.];
const SYNC_GAP: f32 = 4.;
const SYNC_WIDTH: f32 = SYNC_WIDTHS[0] + SYNC_WIDTHS[1] + SYNC_WIDTHS[2] + 2. * SYNC_GAP;
const SYNC_HEIGHT: f32 = 26.;

pub(super) fn fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let root = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Windows"))
        .join("Fonts");
    for (file, name, family) in [
        ("arial.ttf", "Arial", egui::FontFamily::Proportional),
        ("cour.ttf", "Courier New", egui::FontFamily::Monospace),
    ] {
        if let Ok(bytes) = std::fs::read(root.join(file)) {
            fonts
                .font_data
                .insert(name.into(), egui::FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.into());
        }
    }
    ctx.set_fonts(fonts);
    icon_texture(ctx);
    credits::load_marks(ctx);
    ctx.style_mut_of(egui::Theme::Light, |s| {
        s.spacing.item_spacing = Vec2::new(6., 4.);
        s.text_styles
            .insert(egui::TextStyle::Body, FontId::proportional(13.));
        s.text_styles
            .insert(egui::TextStyle::Button, FontId::proportional(13.));
    });
}

#[derive(Clone, Copy)]
enum Icon {
    Sidebar(bool),
    Undo,
    Redo,
    New,
    Open,
    Save,
    Pencil,
    Gear,
    List,
    Home,
    Flask,
    Sliders,
    Expand(bool),
    Clear,
    Down,
    Up,
    Play(bool),
    Pause,
    Stop,
    Step(bool),
    Fast(bool),
}
fn icon_texture(ctx: &egui::Context) {
    let bytes = include_bytes!("../../assets/toolbar-icons.png");
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .expect("embedded icon atlas metadata");
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .expect("embedded icon atlas size")
    ];
    let info = reader
        .next_frame(&mut pixels)
        .expect("embedded icon atlas pixels");
    let texture = ctx.load_texture(
        "CNC Viewer icons",
        egui::ColorImage::from_rgba_unmultiplied(
            [info.width as usize, info.height as usize],
            &pixels[..info.buffer_size()],
        ),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("nc-icons"), texture));
}

fn icon(ui: &egui::Ui, rect: Rect, icon: Icon, color: Color32) {
    let arrow = match icon {
        Icon::Sidebar(open) => Some(Vec2::new(if open { -1. } else { 1. }, 0.)),
        Icon::Up => Some(Vec2::new(0., -1.)),
        Icon::Down => Some(Vec2::new(0., 1.)),
        _ => None,
    };
    if let Some(direction) = arrow {
        let center = rect.center();
        let across = Vec2::new(-direction.y, direction.x) * 4.5;
        for offset in [-3., 3.] {
            let base = center + direction * (offset - 2.);
            let tip = center + direction * (offset + 2.5);
            ui.painter().add(egui::Shape::line(
                vec![base + across, tip, base - across],
                Stroke::new(1.8, color),
            ));
        }
        return;
    }
    let Some(texture) =
        ui.data(|data| data.get_temp::<egui::TextureHandle>(egui::Id::unique("nc-icons")))
    else {
        return;
    };
    // Order follows the eight-column atlas.
    let (index, size) = match icon {
        Icon::Sidebar(_) | Icon::Down | Icon::Up => unreachable!(),
        Icon::Undo => (1, 17.),
        Icon::Redo => (2, 17.),
        Icon::New => (3, 22.),
        Icon::Open => (4, 24.),
        Icon::Save => (5, 20.),
        Icon::Pencil => (8, 20.),
        Icon::Gear => (9, 17.),
        Icon::List => (10, 17.),
        Icon::Home => (11, 29.),
        Icon::Flask => (12, 20.),
        Icon::Sliders => (13, 21.),
        Icon::Expand(false) => (14, 19.),
        Icon::Expand(true) => (15, 19.),
        Icon::Clear => (16, 17.),
        Icon::Play(_) => (18, 18.),
        Icon::Pause => (19, 17.),
        Icon::Stop => (20, 17.),
        Icon::Step(false) => (21, 16.),
        Icon::Step(true) => (22, 16.),
        Icon::Fast(false) => (23, 20.),
        Icon::Fast(true) => (24, 20.),
    };
    let size = f32::min(size, rect.width().min(rect.height()) + 2.);
    let mut uv = Rect::from_min_size(
        Pos2::new((index % 8) as f32 / 8., (index / 8) as f32 / 4.),
        Vec2::new(1. / 8., 1. / 4.),
    );
    if matches!(icon, Icon::Play(false)) {
        std::mem::swap(&mut uv.min.x, &mut uv.max.x);
    }
    ui.painter().image(
        texture.id(),
        Rect::from_center_size(rect.center(), Vec2::splat(size)),
        uv,
        color,
    );
}
fn button(
    ui: &mut egui::Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    kind: Icon,
    color: Color32,
    tip: &str,
) -> bool {
    let response = button_response(ui, rect, id, tip);
    icon(ui, rect.shrink(4.), kind, color);
    response.clicked()
}
fn button_response(
    ui: &mut egui::Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    name: &str,
) -> egui::Response {
    let response = ui.interact(rect, ui.scope_id().with(id), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(rect.shrink(2.), 2., Color32::from_black_alpha(18));
    }
    response.on_hover_text(name)
}
fn label(ui: &egui::Ui, p: Pos2, text: impl ToString, size: f32, color: Color32) {
    ui.painter().text(
        p,
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(size),
        color,
    );
}
fn paint_action(
    ui: &egui::Ui,
    rect: Rect,
    colors: ([u8; 3], [u8; 3]),
    enabled: bool,
    emphasis: bool,
) {
    if enabled {
        let [r, g, b] = if emphasis { colors.1 } else { colors.0 };
        ui.painter().add(
            egui::epaint::Shadow {
                offset: [0, 2],
                blur: 7,
                spread: 0,
                color: Color32::from_black_alpha(53),
            }
            .as_shape(rect, 3.),
        );
        ui.painter()
            .rect_filled(rect, 3., Color32::from_rgba_unmultiplied(r, g, b, 180));
    } else {
        ui.painter().rect_filled(rect, 3., Color32::from_gray(226));
    }
}
fn green_action(
    ui: &mut egui::Ui,
    rect: Rect,
    id: egui::Id,
    caption: &str,
    enabled: bool,
) -> egui::Response {
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, caption));
    paint_action(
        ui,
        rect,
        PLOT_COLORS,
        enabled,
        response.hovered() || response.has_focus(),
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        caption,
        FontId::proportional(12.),
        if enabled {
            Color32::WHITE
        } else {
            DISABLED_ACTION_TEXT
        },
    );
    response
}
fn sync_choice(
    ui: &mut egui::Ui,
    rect: Rect,
    caption: &str,
    id: egui::Id,
    enabled: bool,
    selected: bool,
) -> bool {
    let response = ui.interact(
        rect,
        id,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, enabled, selected, caption)
    });
    paint_action(
        ui,
        rect,
        GRAY_ACTION_COLORS,
        enabled,
        response.hovered() || response.has_focus(),
    );
    let foreground = if enabled {
        Color32::WHITE
    } else {
        DISABLED_ACTION_TEXT
    };
    let indicator = Pos2::new(rect.min.x + 13., rect.center().y);
    ui.painter()
        .circle_stroke(indicator, 4.5, Stroke::new(1., foreground));
    if selected {
        ui.painter().circle_filled(indicator, 2.5, foreground);
    }
    label(
        ui,
        Pos2::new(rect.min.x + 23., rect.center().y),
        caption,
        12.,
        foreground,
    );
    enabled && response.clicked()
}
fn child(ui: &mut egui::Ui, rect: Rect, id: &str) -> egui::Ui {
    let mut c = ui.new_child(
        UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    c.set_clip_rect(rect);
    c
}

impl App {
    pub(super) fn near_view(&mut self, ui: &mut egui::Ui, mut tabs: Option<&mut tabs::Strip<'_>>) {
        let full = ui.max_rect();
        let tabbed = tabs.is_some();
        let toolbar_bottom = full.min.y + TOOLBAR_HEIGHT;
        let content_top = toolbar_bottom + if tabbed { tabs::HEIGHT } else { 0. };
        let plot = Rect::from_min_max(Pos2::new(full.min.x, content_top), full.max);
        let left = if self.settings.editor_open {
            self.settings.left_width.clamp(
                MIN_SIDEBAR_WIDTH,
                (full.width() - 220.).max(MIN_SIDEBAR_WIDTH),
            )
        } else {
            0.
        };
        let mut scene = child(ui, plot, "scene");
        self.canvas(&mut scene, left);
        self.view_cube(ui, plot);
        self.header(ui, full, tabs.as_deref_mut());
        if let Some(tabs) = tabs {
            tabs.show(
                ui,
                Rect::from_min_max(
                    Pos2::new(full.min.x, toolbar_bottom),
                    Pos2::new(full.max.x, content_top),
                ),
            );
        }
        if left > 0. {
            self.sidebar(
                ui,
                Rect::from_min_max(plot.min, Pos2::new(plot.min.x + left, plot.max.y)),
                tabbed,
            );
        }
        let sidebar_toggle = Rect::from_min_size(plot.min + Vec2::new(2., 1.), Vec2::splat(24.));
        if !self.settings.editor_open {
            ui.painter().rect_filled(sidebar_toggle, 2., Color32::WHITE);
        }
        if button(
            ui,
            sidebar_toggle,
            "toggle-sidebar",
            Icon::Sidebar(self.settings.editor_open),
            INK,
            if self.settings.editor_open {
                "Hide sidebar"
            } else {
                "Show sidebar"
            },
        ) {
            self.settings.editor_open = !self.settings.editor_open;
        }
        let status_area = Rect::from_min_max(plot.min + Vec2::new(left + 6., 0.), plot.max);
        ui.painter().with_clip_rect(status_area).text(
            Pos2::new(full.max.x - 12., plot.min.y + 14.),
            Align2::RIGHT_CENTER,
            &self.status,
            FontId::proportional(12.),
            INK,
        );
        self.playback(ui, plot, left);
        if self.editor_options {
            let mut open_find = false;
            egui::Window::new("Editor options")
                .id(self.local_id("editor-options-window"))
                .open(&mut self.editor_options)
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.add(
                        egui::Slider::new(&mut self.settings.font_size, 9. ..=24.)
                            .text("Font size"),
                    );
                    ui.horizontal(|ui| {
                        ui.label("Theme");
                        egui::ComboBox::from_id_salt("editor-theme")
                            .selected_text(if self.settings.dark { "Dark" } else { "Light" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut self.settings.dark, false, "Light");
                                ui.selectable_value(&mut self.settings.dark, true, "Dark");
                            });
                    });
                    open_find = ui.button("Find / replace").clicked();
                });
            if open_find {
                self.open_find(ui.ctx(), false);
                self.editor_options = false;
            }
        }
    }
    fn header(&mut self, ui: &mut egui::Ui, full: Rect, tabs: Option<&mut tabs::Strip<'_>>) {
        let y = full.min.y;
        let x = full.min.x;
        let center_y = y + TOOLBAR_HEIGHT / 2.;
        let band = Rect::from_min_size(full.min, Vec2::new(full.width(), TOOLBAR_HEIGHT));
        ui.painter().rect_filled(band, 0., BANNER_GREEN);
        ui.painter().rect_filled(
            Rect::from_min_max(band.left_bottom() - Vec2::new(0., 1.), band.max),
            0.,
            Color32::from_rgb(89, 160, 117),
        );
        let buttons = [
            (19., Icon::Undo, "Undo"),
            (51., Icon::Redo, "Redo"),
            (90., Icon::New, "New file"),
            (124., Icon::Open, "Open file"),
            (158., Icon::Save, "Save · Ctrl+S"),
        ];
        for (i, (cx, kind, tip)) in buttons.into_iter().enumerate() {
            if button(
                ui,
                Rect::from_center_size(Pos2::new(x + cx, center_y), Vec2::splat(32.)),
                ("toolbar", i),
                kind,
                Color32::WHITE,
                tip,
            ) {
                match i {
                    0 | 1 => self.edit_history(ui.ctx(), i == 1),
                    2 => self.request_document_action(DocumentAction::New),
                    3 => self.open_dialog(),
                    4 => self.save(false),
                    _ => {}
                }
            }
        }
        for dx in [71., 176.] {
            ui.painter().line_segment(
                [Pos2::new(x + dx, y + 7.), Pos2::new(x + dx, y + 29.)],
                Stroke::new(1., Color32::from_white_alpha(85)),
            );
        }
        if let Some(tabs) = tabs {
            let center = Pos2::new(x + 194., center_y);
            let response = button_response(
                ui,
                Rect::from_center_size(center, Vec2::splat(32.)),
                "file-history",
                "File history and samples",
            );
            let stroke = Stroke::new(1.8, Color32::WHITE);
            ui.painter().circle_stroke(center, 8., stroke);
            ui.painter().add(egui::Shape::line(
                vec![
                    center - Vec2::new(0., 4.7),
                    center,
                    center + Vec2::new(3.6, 1.9),
                ],
                stroke,
            ));
            tabs.history_menu(&response);
        }
        credits::show(ui, band, x + 218.);
    }
    fn sidebar(&mut self, ui: &mut egui::Ui, rect: Rect, tabbed: bool) {
        ui.painter().rect_filled(rect, 0., Color32::WHITE);
        for i in 0..4 {
            ui.painter().rect_filled(
                Rect::from_min_max(
                    Pos2::new(rect.max.x + i as f32, rect.min.y),
                    Pos2::new(rect.max.x + i as f32 + 1., rect.max.y),
                ),
                0.,
                Color32::from_black_alpha(15 - i * 3),
            );
        }
        let rect = Rect::from_min_max(rect.min + Vec2::new(SIDEBAR_GUTTER, 0.), rect.max);
        let head = Rect::from_min_size(rect.min, Vec2::new(rect.width(), SECTION_HEIGHT));
        let mut title_rect = section_title_rect(head);
        let multiple_sources = self.documents.len() > 1;
        let title = if tabbed { "Editor" } else { &self.selected };
        let show_source_name = multiple_sources && tabbed;
        let mut below_header = head.max.y;
        let sync = self
            .disk
            .files
            .get(&self.selected)
            .is_some_and(disk::File::out_of_sync)
            .then(|| {
                let title_width = ui
                    .painter()
                    .layout_no_wrap(
                        self.selected.clone(),
                        FontId::proportional(SECTION_TITLE_SIZE),
                        INK,
                    )
                    .size()
                    .x
                    + if multiple_sources {
                        SOURCE_PICKER_WIDTH
                    } else {
                        0.
                    };
                if title_rect.width() >= title_width.min(114.) + SYNC_WIDTH + 8. {
                    let area = Rect::from_min_size(
                        Pos2::new(
                            title_rect.max.x - SYNC_WIDTH,
                            head.center().y - SYNC_HEIGHT / 2.,
                        ),
                        Vec2::new(SYNC_WIDTH, SYNC_HEIGHT),
                    );
                    title_rect.max.x = area.min.x - 8.;
                    area
                } else {
                    below_header += SYNC_HEIGHT + 6.;
                    Rect::from_min_size(
                        Pos2::new(
                            title_rect.min.x.min(head.max.x - SYNC_WIDTH - 6.),
                            head.max.y + 1.,
                        ),
                        Vec2::new(SYNC_WIDTH, SYNC_HEIGHT),
                    )
                }
            });
        let header = section_header(
            ui,
            head,
            "code-header",
            Icon::Pencil,
            title,
            self.settings.code_open,
        );
        let picker_right = title_rect.max.x;
        if multiple_sources {
            title_rect.max.x -= SOURCE_PICKER_WIDTH;
        }
        let title_right = section_title(ui, title_rect, title);
        if multiple_sources {
            let text =
                ui.painter()
                    .layout_no_wrap(self.selected.clone(), FontId::proportional(12.), INK);
            let width = if show_source_name {
                (text.size().x + 25.).min(picker_right - title_right - 2.)
            } else {
                SOURCE_PICKER_WIDTH - 2.
            };
            let picker = Rect::from_min_size(
                Pos2::new(title_right + 2., title_rect.min.y),
                Vec2::new(width, title_rect.height()),
            );
            let source_button = button_response(ui, picker, "choose-source", "Choose source");
            let p = Pos2::new(picker.max.x - 11., picker.center().y);
            if show_source_name {
                let name_area = Rect::from_min_max(
                    picker.min + Vec2::new(6., 0.),
                    picker.max - Vec2::new(19., 0.),
                );
                ui.painter().with_clip_rect(name_area).galley(
                    Pos2::new(name_area.min.x, picker.center().y - text.size().y / 2.),
                    text,
                    INK,
                );
            }
            ui.painter().add(egui::Shape::convex_polygon(
                vec![
                    p + Vec2::new(-4., -2.),
                    p + Vec2::new(4., -2.),
                    p + Vec2::new(0., 2.),
                ],
                INK,
                Stroke::NONE,
            ));
            egui::Popup::menu(&source_button).show(|ui| {
                for name in self.documents.keys() {
                    if ui
                        .selectable_value(&mut self.selected, name.clone(), name)
                        .clicked()
                    {
                        ui.close();
                    }
                }
            });
        }
        header.context_menu(|ui| {
            if ui.button("Add subprograms...").clicked() {
                self.add_libraries();
                ui.close();
            }
            if ui.button("Use this source as main").clicked() {
                self.main = self.selected.clone();
                self.replot();
                ui.close();
            }
            if ui.button("Load machine setup...").clicked() {
                self.load_machine_setup();
                ui.close();
            }
            if !self
                .program_setup
                .as_object()
                .is_none_or(|setup| setup.is_empty())
                && ui.button("Reset machine setup").clicked()
            {
                self.program_setup = serde_json::json!({});
                self.replot();
                ui.close();
            }
            if ui.button("Save").clicked() {
                self.save(false);
                ui.close();
            }
            if ui.button("Save as...").clicked() {
                self.save(true);
                ui.close();
            }
        });
        if let Some(area) = sync {
            self.disk_conflict(ui, area);
        }
        let clicked_sync = header
            .interact_pointer_pos()
            .is_some_and(|p| sync.is_some_and(|area| area.contains(p)));
        if header.clicked() && !clicked_sync {
            self.settings.code_open = !self.settings.code_open;
        }
        let machine_height = if self.settings.machine_open {
            105.
        } else {
            SECTION_HEIGHT
        };
        let dro_height = if self.settings.dro_open {
            154.
        } else {
            SECTION_HEIGHT
        };
        let code = Rect::from_min_max(
            Pos2::new(rect.min.x, below_header),
            Pos2::new(
                rect.max.x,
                if self.settings.code_open {
                    rect.max.y - machine_height - dro_height
                } else {
                    below_header
                },
            ),
        );
        if self.settings.code_open {
            self.code(ui, code);
        }
        let dro = Rect::from_min_size(
            Pos2::new(rect.min.x, code.max.y),
            Vec2::new(rect.width(), dro_height),
        );
        if section(
            ui,
            dro,
            "dro",
            Icon::List,
            "Digital Read Out",
            self.settings.dro_open,
        ) {
            self.settings.dro_open = !self.settings.dro_open;
        }
        if self.settings.dro_open {
            let (axes, inch) = self.dro();
            for row in 0..2 {
                for col in 0..3 {
                    let x = dro.min.x + col as f32 * dro.width() / 3.;
                    let y = dro.min.y + 48. + row as f32 * 45.;
                    let axis = ["X", "Y", "Z", "A", "B", "C"][row * 3 + col];
                    label(ui, Pos2::new(x + 18., y), axis, 12., INK);
                    label(
                        ui,
                        Pos2::new(x + 18., y + 20.),
                        format!("{:0.3}", axes[row * 3 + col]),
                        15.,
                        INK,
                    );
                }
            }
            label(
                ui,
                Pos2::new(dro.max.x - 90., dro.max.y - 10.),
                if inch { "Inches" } else { "Millimeters" },
                10.,
                INK,
            );
        }
        let machine = Rect::from_min_size(
            Pos2::new(rect.min.x, dro.max.y),
            Vec2::new(rect.width(), machine_height),
        );
        if section(
            ui,
            machine,
            "machine",
            Icon::Gear,
            "Machine Options",
            self.settings.machine_open,
        ) {
            self.settings.machine_open = !self.settings.machine_open;
        }
        if self.settings.machine_open {
            let mut c = child(
                ui,
                Rect::from_min_max(
                    machine.min + Vec2::new(14., 38.),
                    machine.max - Vec2::new(12., 2.),
                ),
                "machine-options",
            );
            toggle(
                &mut c,
                "Plot Orientation: Vertical (Z-Up)",
                &mut self.settings.z_up,
            );
            if toggle(
                &mut c,
                "Diameter Mode (Lathe)",
                &mut self.settings.diameter_mode,
            ) {
                self.replot();
            }
        }
        let resize = Rect::from_min_max(
            Pos2::new(rect.max.x - 3., rect.min.y),
            Pos2::new(rect.max.x + 3., rect.max.y),
        );
        let response = ui
            .interact(resize, ui.scope_id().with("sidebar-resize"), Sense::drag())
            .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if response.dragged() {
            self.settings.left_width =
                (self.settings.left_width + ui.input(|i| i.pointer.delta().x)).clamp(
                    MIN_SIDEBAR_WIDTH,
                    (ui.max_rect().width() - 220.).max(MIN_SIDEBAR_WIDTH),
                );
        }
    }
    fn disk_conflict(&mut self, ui: &mut egui::Ui, rect: Rect) {
        use disk::Choice;
        let name = self.selected.clone();
        let can_reload = self
            .disk
            .files
            .get(&name)
            .is_some_and(|file| matches!(file.observed, disk::Version::Text(..)));
        let selection = self
            .conflict_choice
            .as_ref()
            .and_then(|(source, choice)| (source == &name).then_some(*choice));
        let mut x = rect.min.x;
        for (index, (choice, caption, enabled)) in [
            (Choice::Reload, "Reload", can_reload),
            (Choice::Overwrite, "Overwrite", true),
        ]
        .into_iter()
        .enumerate()
        {
            let area = Rect::from_min_size(
                Pos2::new(x, rect.min.y),
                Vec2::new(SYNC_WIDTHS[index], SYNC_HEIGHT),
            );
            x = area.max.x + SYNC_GAP;
            let selected = selection == Some(choice);
            if sync_choice(
                ui,
                area,
                caption,
                self.local_id(("sync-choice", caption)),
                enabled,
                selected,
            ) {
                self.conflict_choice = (!selected).then(|| (name.clone(), choice));
            }
        }
        let apply = Rect::from_min_max(Pos2::new(x, rect.min.y), rect.max);
        if green_action(
            ui,
            apply,
            self.local_id("apply-sync"),
            "Apply sync",
            selection.is_some(),
        )
        .clicked()
        {
            if let Some(choice) = selection {
                self.apply_disk_choice(&name, choice, ui.ctx());
            }
        }
    }
    fn code(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let max_variables = (rect.width() - 144.).max(96.);
        let expanded = self.settings.show_variables;
        let variables = if expanded {
            self.settings.variables_width.clamp(96., max_variables)
        } else {
            24.
        };
        let code = Rect::from_min_max(rect.min, Pos2::new(rect.max.x - variables, rect.max.y));
        let vars = Rect::from_min_max(Pos2::new(code.max.x, rect.min.y), rect.max);
        ui.painter().rect_filled(vars, 0., Color32::from_gray(248));
        let toggle_rect = if expanded {
            Rect::from_min_size(vars.min, Vec2::new(vars.width(), 26.))
        } else {
            vars
        };
        let toggle = button_response(
            ui,
            toggle_rect,
            "variables-toggle",
            if expanded {
                "Collapse variables"
            } else {
                "Expand variables"
            },
        );
        icon(
            ui,
            Rect::from_min_size(vars.min, Vec2::splat(24.)).shrink(4.),
            Icon::Sidebar(!expanded),
            INK,
        );
        if toggle.clicked() {
            self.settings.show_variables = !expanded;
        }
        if expanded {
            label(ui, vars.min + Vec2::new(26., 12.), "Variables", 12., INK);
            let mut v = child(
                ui,
                Rect::from_min_max(vars.min + Vec2::new(5., 28.), vars.max),
                "variables",
            );
            egui::ScrollArea::both().show(&mut v, |ui| {
                egui::Grid::new("variables-grid")
                    .striped(true)
                    .min_col_width(47.)
                    .show(ui, |ui| {
                        for number in &self.model.variable_numbers {
                            ui.monospace(format!("#{number}"));
                            let value = self
                                .model
                                .executions
                                .get(self.step)
                                .and_then(|s| s.variables.get(number))
                                .copied()
                                .flatten();
                            ui.monospace(
                                value
                                    .map(|n| {
                                        format!("{n:0.6}")
                                            .trim_end_matches('0')
                                            .trim_end_matches('.')
                                            .to_owned()
                                    })
                                    .unwrap_or_else(|| "-".into()),
                            );
                            ui.end_row();
                        }
                    });
            });
        } else {
            let galley =
                ui.painter()
                    .layout_no_wrap("Variables".into(), FontId::proportional(12.), INK);
            ui.painter().add(
                egui::epaint::TextShape::new(
                    Pos2::new(vars.center().x + 6., vars.min.y + 26.),
                    galley,
                    INK,
                )
                .with_angle(std::f32::consts::FRAC_PI_2),
            );
        }
        let mut editor = child(ui, code, "code");
        let find_height = if self.find.open {
            self.find_bar(&mut editor, code)
        } else {
            0.
        };
        let edit_rect = Rect::from_min_max(code.min + Vec2::new(0., find_height), code.max);
        let mut e = child(&mut editor, edit_rect, "text");
        e.style_mut().spacing.scroll.kinetic.rubber_band = false;
        let id = self.editor_id();
        let size = self.settings.font_size;
        let dark = self.settings.dark;
        if dark {
            e.painter()
                .rect_filled(edit_rect, 0., Color32::from_rgb(39, 40, 34));
        }
        let line_height = (size + 2.).max(14.);
        let selected = self.selected.clone();
        let notes = diagnostics::for_source(&self.diagnostics, &selected);
        let line = self.selected_line;
        let text = self.documents.entry(selected.clone()).or_default();
        let mut scroll = egui::ScrollArea::both()
            .id_salt(("code-scroll", &self.project_id, &selected))
            .auto_shrink([false, false]);
        if let Some(line) = self.scroll_line.take() {
            scroll = scroll.vertical_scroll_offset(
                ((line.saturating_sub(1)) as f32 * line_height - edit_rect.height() / 2.).max(0.),
            );
            if !self.running {
                let start = text
                    .split_inclusive('\n')
                    .take(line.saturating_sub(1))
                    .map(|row| row.chars().count())
                    .sum::<usize>();
                let mut state =
                    egui::text_edit::TextEditState::load(e.ctx(), id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(start),
                    )));
                state.store(e.ctx(), id);
            }
        }
        if let Some(line) = page_editor(
            e.ctx(),
            id,
            text,
            (edit_rect.height() / line_height).max(2.) as usize - 1,
        ) {
            scroll = scroll.vertical_scroll_offset(
                ((line.saturating_sub(1)) as f32 * line_height - edit_rect.height() / 2.).max(0.),
            );
        }
        let previous_cursor = egui::text_edit::TextEditState::load(e.ctx(), id)
            .and_then(|state| state.cursor.char_range())
            .map(|range| range.primary.index);
        let mut changed = false;
        scroll.show_viewport(&mut e, |ui, viewport| {
            let origin = ui.cursor().min;
            let lines = text.bytes().filter(|&byte| byte == b'\n').count() + 1;
            let gutter = Rect::from_min_size(
                origin,
                Vec2::new(47., (lines as f32 * line_height).max(edit_rect.height())),
            );
            ui.painter().rect_filled(
                gutter,
                0.,
                if dark {
                    Color32::from_rgb(47, 49, 41)
                } else {
                    Color32::from_gray(235)
                },
            );
            if line > 0 {
                let row = Rect::from_min_size(
                    origin + Vec2::new(0., (line - 1) as f32 * line_height),
                    Vec2::new(code.width().max(ui.available_width()), line_height),
                );
                ui.painter().rect_filled(
                    row,
                    0.,
                    if dark {
                        Color32::from_rgb(57, 58, 50)
                    } else {
                        Color32::from_gray(236)
                    },
                );
                ui.painter().rect_filled(
                    Rect::from_min_size(row.min, Vec2::new(47., line_height)),
                    0.,
                    if dark {
                        Color32::from_rgb(66, 67, 58)
                    } else {
                        Color32::from_gray(220)
                    },
                );
            }
            let diagnostic_painter = ui.painter().with_clip_rect(edit_rect);
            let diagnostic_background =
                (!notes.is_empty()).then(|| diagnostic_painter.add(egui::Shape::Noop));
            let clip = ui.clip_rect();
            let first = ((clip.min.y - origin.y) / line_height).max(0.) as usize;
            let last = (((clip.max.y - origin.y) / line_height).ceil() as usize + 1).min(lines);
            for n in first..last {
                ui.painter().text(
                    origin + Vec2::new(39., (n as f32 + 0.5) * line_height),
                    Align2::RIGHT_CENTER,
                    (n + 1).to_string(),
                    FontId::monospace(size),
                    if dark {
                        Color32::from_gray(172)
                    } else {
                        Color32::from_gray(80)
                    },
                );
            }
            let mut body = ui.new_child(
                UiBuilder::new()
                    .id_salt("editor-body")
                    .max_rect(Rect::from_min_max(
                        origin + Vec2::new(49., 0.),
                        Pos2::new(origin.x + code.width(), origin.y + gutter.height()),
                    ))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let cache = &mut self.highlight_cache;
            let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _width: f32| {
                let dpi = ui.ctx().pixels_per_point().to_bits();
                let source = buffer.as_str();
                let rows = (source.len() > 131_072).then(|| {
                    (
                        (first / 128).saturating_sub(1) * 128,
                        (last / 128 + 2) * 128,
                    )
                });
                if let Some(cached) = cache.get(&selected) {
                    if cached.size == size.to_bits()
                        && cached.dpi == dpi
                        && cached.dark == dark
                        && cached.rows == rows
                        && cached.text == source
                    {
                        return cached.galley.clone();
                    }
                }
                let galley = ui
                    .fonts_mut(|f| f.layout_job(highlight(source, size, line_height, dark, rows)));
                cache.insert(
                    selected.clone(),
                    Highlight {
                        text: source.to_owned(),
                        size: size.to_bits(),
                        dpi,
                        dark,
                        rows,
                        galley: galley.clone(),
                    },
                );
                galley
            };
            let search_painter = body.painter().clone();
            let search_shape = (self.find.open
                && self
                    .find
                    .results
                    .as_ref()
                    .is_some_and(|r| !r.ranges.is_empty()))
            .then(|| search_painter.add(egui::Shape::Noop));
            let output = egui::TextEdit::multiline(text)
                .id(id)
                .font(FontId::monospace(size))
                .code_editor()
                .frame(egui::Frame::NONE)
                .margin(Vec2::ZERO)
                .desired_width(f32::INFINITY)
                .desired_rows(1)
                .layouter(&mut layouter)
                .show(&mut body);
            // A final newline has a caret row of its own. Keep the preceding row
            // on screen at the lower scroll limit.
            let last = output
                .galley
                .rows
                .len()
                .saturating_sub(1 + usize::from(text.ends_with('\n')));
            let last_top = output.galley.rows.get(last).map_or(0., |row| row.min_y());
            let content = gutter.union(output.response.rect);
            ui.expand_to_include_rect(Rect::from_min_max(
                content.min,
                Pos2::new(
                    content.max.x,
                    content
                        .max
                        .y
                        .max(output.galley_pos.y + last_top + viewport.height()),
                ),
            ));
            if output.response.changed() {
                changed = true;
                return;
            }
            if let Some(background) = diagnostic_background {
                diagnostics::paint(
                    ui,
                    &diagnostic_painter,
                    background,
                    &output,
                    notes,
                    size,
                    dark,
                );
            }
            if let (Some(shape), Some(results)) = (search_shape, &self.find.results) {
                search_painter.set(
                    shape,
                    find::match_highlights(results, text, &output, &search_painter, dark),
                );
                if std::mem::take(&mut self.find.reveal) {
                    find::reveal_match(results, text, &output, ui);
                }
            }
            if let Some(range) = output.cursor_range.filter(|_| !self.preview_stale) {
                let caret_moved = output.response.has_focus()
                    && previous_cursor.is_some_and(|index| index != range.primary.index);
                if output.response.clicked() || caret_moved {
                    let line = text
                        .chars()
                        .take(range.primary.index.into())
                        .filter(|c| *c == '\n')
                        .count()
                        + 1;
                    if output.response.clicked() || line != self.selected_line {
                        let mut visits = self
                            .model
                            .executions
                            .iter()
                            .enumerate()
                            .filter(|(_, state)| state.source == selected && state.line == line)
                            .map(|(index, _)| index);
                        let first = visits.next();
                        let next = first
                            .filter(|&index| index >= self.step)
                            .or_else(|| visits.find(|&index| index >= self.step))
                            .or(first);
                        if let Some(index) = next {
                            self.step = index;
                            self.time = self.model.executions[index].elapsed / 1000.;
                            self.selected_line = line;
                            self.last_stop = None;
                            self.running = false;
                        }
                    }
                }
            }
        });
        if changed {
            self.edited(ui.ctx());
        }
        let y = code.max.y - 35.;
        let x = (code.max.x - 146.).max(code.min.x);
        let plot = Rect::from_min_size(Pos2::new(x, y), Vec2::new(54., 30.));
        if green_action(ui, plot, ui.scope_id().with("plot"), "PLOT", true)
            .on_hover_text("Plot · Ctrl+Enter")
            .clicked()
        {
            self.replot();
        }
        let clear = Rect::from_min_size(Pos2::new(x + 62., y), Vec2::new(34., 30.));
        paint_action(
            ui,
            clear,
            ([229, 57, 53], [239, 83, 80]),
            true,
            ui.rect_contains_pointer(clear),
        );
        if button(
            ui,
            clear,
            "clear-plot",
            Icon::Clear,
            Color32::WHITE,
            "Clear toolpath",
        ) {
            self.clear_plot("");
        }
        let options = clear.translate(Vec2::new(42., 0.));
        paint_action(
            ui,
            options,
            GRAY_ACTION_COLORS,
            true,
            ui.rect_contains_pointer(options),
        );
        if button(
            ui,
            options,
            "editor-options",
            Icon::Gear,
            Color32::WHITE,
            "Editor options",
        ) {
            self.editor_options = !self.editor_options;
        }
        let divider = [vars.min, Pos2::new(vars.min.x, vars.max.y)];
        let mut color = Color32::from_gray(221);
        if variables > 24. {
            let response = ui
                .interact(
                    Rect::from_min_max(divider[0], divider[1]).expand2(Vec2::new(4., 0.)),
                    ui.scope_id().with("variables-resize"),
                    Sense::drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if response.hovered() || response.dragged() {
                color = MINT;
            }
            if response.dragged() || response.drag_stopped() {
                if let Some(position) = response.interact_pointer_pos() {
                    self.settings.variables_width =
                        (rect.max.x - position.x).clamp(96., max_variables);
                }
            }
        }
        ui.painter().line_segment(divider, Stroke::new(1., color));
    }
    fn playback(&mut self, ui: &mut egui::Ui, plot: Rect, left: f32) {
        let width = (plot.width() - left - 24.).clamp(220., 720.);
        let x = plot.min.x + (plot.width() + left - width) / 2.;
        let y = plot.max.y - 59.;
        let main = Rect::from_min_size(Pos2::new(x, y), Vec2::new(width - 84., 49.));
        let options = Rect::from_min_size(Pos2::new(main.max.x + 8., y + 14.), Vec2::new(76., 35.));
        for rect in [main, options] {
            ui.painter().add(
                egui::epaint::Shadow {
                    offset: [0, 2],
                    blur: 7,
                    spread: 0,
                    color: Color32::from_black_alpha(76),
                }
                .as_shape(rect, 4.),
            );
            ui.painter()
                .rect_filled(rect, 4., Color32::from_black_alpha(153));
        }
        let number = Rect::from_min_size(main.min + Vec2::new(6., 2.), Vec2::new(56., 13.));
        ui.painter().text(
            number.right_center(),
            Align2::RIGHT_CENTER,
            if self.model.executions.is_empty() {
                0
            } else {
                self.step + 1
            }
            .to_string(),
            FontId::monospace(11.),
            Color32::WHITE,
        );
        let track = Rect::from_min_max(
            Pos2::new(number.max.x + 5., y + 4.),
            Pos2::new(main.max.x - 12., y + 15.),
        );
        let max = max_step(&self.model);
        let r = ui.interact(
            track.expand2(Vec2::new(1., 3.)),
            self.local_id("program-progress"),
            Sense::click_and_drag(),
        );
        r.widget_info(|| {
            egui::WidgetInfo::slider(max > 0, (self.step + 1) as f64, "Program progress")
        });
        if r.clicked() || r.drag_started() {
            r.request_focus();
        }
        if (r.dragged() || r.clicked()) && max > 0 {
            if let Some(p) = r.interact_pointer_pos() {
                self.seek(
                    (((p.x - track.min.x) / track.width()).clamp(0., 1.) * max as f32).round()
                        as usize,
                );
            }
        }
        let cy = track.center().y;
        ui.painter().line_segment(
            [Pos2::new(track.min.x, cy), Pos2::new(track.max.x, cy)],
            Stroke::new(3., Color32::from_gray(153)),
        );
        let pos = track.min.x
            + track.width()
                * if max > 0 {
                    self.step as f32 / max as f32
                } else {
                    0.
                };
        ui.painter().line_segment(
            [Pos2::new(track.min.x, cy), Pos2::new(pos, cy)],
            Stroke::new(3., MINT),
        );
        ui.painter()
            .circle_filled(Pos2::new(pos, cy), 4., Color32::WHITE);
        let cell = main.width() / 7.;
        let icons = [
            Icon::Fast(false),
            Icon::Step(false),
            if self.running && !self.forward {
                Icon::Pause
            } else {
                Icon::Play(false)
            },
            Icon::Stop,
            if self.running && self.forward {
                Icon::Pause
            } else {
                Icon::Play(true)
            },
            Icon::Step(true),
            Icon::Fast(true),
        ];
        let tips = [
            "Faster reverse",
            "Previous block",
            "Reverse / pause",
            "Stop",
            "Play / pause",
            "Next block",
            "Faster forward",
        ];
        for i in 0..7 {
            let rect = Rect::from_min_size(
                Pos2::new(x + i as f32 * cell, y + 18.),
                Vec2::new(cell, 29.),
            );
            if button(
                ui,
                rect,
                ("play", i),
                icons[i],
                if self.running && ((i == 2 && !self.forward) || (i == 4 && self.forward)) {
                    MINT
                } else {
                    Color32::WHITE
                },
                tips[i],
            ) {
                match i {
                    0 | 6 => {
                        self.faster(i == 6);
                    }
                    1 => self.seek(self.step.saturating_sub(1)),
                    2 | 4 => {
                        let forward = i == 4;
                        if self.running && self.forward == forward {
                            self.running = false;
                        } else {
                            self.play(forward);
                        }
                    }
                    3 => {
                        self.running = false;
                        self.seek(0);
                    }
                    5 => self.seek(self.step + 1),
                    _ => {}
                }
            }
        }
        for (i, kind) in [Icon::Flask, Icon::Sliders, Icon::Expand(self.fullscreen)]
            .into_iter()
            .enumerate()
        {
            let rect = Rect::from_min_size(
                options.min + Vec2::new(i as f32 * 25., 0.),
                Vec2::new(25., 35.),
            );
            if button(
                ui,
                rect,
                ("options", i),
                kind,
                Color32::WHITE,
                ["Mouse controls", "Backplot options", "Fullscreen"][i],
            ) {
                if i == 2 {
                    self.fullscreen = !self.fullscreen;
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                } else {
                    self.popup = if self.popup == i as u8 + 1 {
                        0
                    } else {
                        i as u8 + 1
                    };
                }
            }
        }
        if self.popup > 0 {
            let h = if self.popup == 2 { 213. } else { 205. };
            let rect = Rect::from_min_size(
                Pos2::new((options.max.x - 285.).max(plot.min.x + 4.), y - h - 8.),
                Vec2::new(285., h),
            );
            let outside_click = ui.input(|input| {
                input.pointer.primary_clicked()
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|pos| !rect.contains(pos) && !options.contains(pos))
            });
            if outside_click {
                self.popup = 0;
                return;
            }
            let background = Color32::from_black_alpha(195);
            let p = ui.painter();
            p.add(
                egui::epaint::Shadow {
                    offset: [0, 2],
                    blur: 8,
                    spread: 0,
                    color: Color32::from_black_alpha(82),
                }
                .as_shape(rect, 4.),
            );
            p.rect_filled(rect, 4., background);
            let anchor = options.min.x + (self.popup as f32 - 0.5) * 25.;
            p.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(anchor - 7., rect.max.y - 0.5),
                    Pos2::new(anchor + 7., rect.max.y - 0.5),
                    Pos2::new(anchor, rect.max.y + 7.),
                ],
                background,
                Stroke::NONE,
            ));
            let mut pop = child(ui, rect.shrink2(Vec2::new(9., 0.)), "backplot-popup");
            pop.visuals_mut().override_text_color = Some(Color32::WHITE);
            pop.painter().text(
                Pos2::new(rect.center().x, rect.min.y + 15.),
                Align2::CENTER_CENTER,
                if self.popup == 2 {
                    "Backplot options"
                } else {
                    "Mouse controls"
                },
                FontId::proportional(15.),
                Color32::WHITE,
            );
            let separator = rect.min.y + 28.;
            pop.painter().line_segment(
                [
                    Pos2::new(rect.min.x + 8., separator),
                    Pos2::new(rect.max.x - 8., separator),
                ],
                Stroke::new(1., Color32::from_rgb(212, 55, 50)),
            );
            pop.add_space(31.);
            pop.spacing_mut().item_spacing.y = 4.;
            if self.popup == 2 {
                measured_toggle(
                    &mut pop,
                    "Show toolbit path",
                    &mut self.settings.show_toolbit_path,
                    &mut self.settings.diameter,
                    0.01..=10000.,
                    0.1,
                    "mm",
                );
                measured_toggle(
                    &mut pop,
                    "Feed timing",
                    &mut self.settings.feed_timed,
                    &mut self.settings.speed,
                    0.1..=1024.,
                    1.,
                    "×",
                );
                toggle(&mut pop, "Show grid", &mut self.settings.show_grid);
                toggle(
                    &mut pop,
                    "Show rapid movement",
                    &mut self.settings.show_rapid,
                );
                toggle(
                    &mut pop,
                    "Show toolpath points",
                    &mut self.settings.show_points,
                );
                toggle(
                    &mut pop,
                    "Hide toolpath after position",
                    &mut self.settings.hide_after,
                );
                toggle(
                    &mut pop,
                    "Enable 5 Axis movement",
                    &mut self.settings.five_axis,
                );
            } else {
                toggle(&mut pop, "Enable raycasting", &mut self.settings.raycast);
                #[cfg(windows)]
                toggle(&mut pop, "Enable 3D Mouse", &mut self.settings.space_mouse);
                for (index, name) in ["Pan", "Zoom", "Orbit"].into_iter().enumerate() {
                    pop.horizontal(|ui| {
                        ui.label(name);
                        let old = self.settings.mouse[index];
                        ui.scope(|ui| {
                            let visuals = ui.visuals_mut();
                            visuals.override_text_color = Some(TOOL_INK);
                            visuals.widgets.inactive.weak_bg_fill = Color32::from_gray(238);
                            visuals.widgets.hovered.weak_bg_fill = Color32::WHITE;
                            visuals.widgets.active.weak_bg_fill = Color32::WHITE;
                            visuals.window_fill = Color32::from_gray(247);
                            egui::ComboBox::from_id_salt(("mouse", index))
                                .selected_text(["Left", "Middle", "Right"][old])
                                .show_ui(ui, |ui| {
                                    for (button, name) in
                                        ["Left", "Middle", "Right"].into_iter().enumerate()
                                    {
                                        if ui.selectable_label(old == button, name).clicked() {
                                            if let Some(other) = self
                                                .settings
                                                .mouse
                                                .iter()
                                                .position(|b| *b == button)
                                            {
                                                self.settings.mouse[other] = old;
                                            }
                                            self.settings.mouse[index] = button;
                                        }
                                    }
                                });
                        });
                    });
                }
                pop.label("Wheel: zoom at pointer");
            }
        }
    }
    fn view_cube(&mut self, ui: &mut egui::Ui, plot: Rect) {
        let center = Pos2::new(plot.max.x - 72., plot.min.y + 132.);
        let rect = Rect::from_center_size(center, Vec2::splat(100.));
        let response = ui.interact(
            rect,
            ui.scope_id().with("view-cube"),
            Sense::click_and_drag(),
        );
        self.orbit_drag(&response, PointerButton::Secondary);
        let hit = {
            let project = cube::projector(&self.camera, center);
            let faces = cube::visible(&self.camera, &project);
            let hovered = response
                .hover_pos()
                .and_then(|position| faces.iter().position(|face| face.contains(position)));
            let alpha = if response.hovered() { 255 } else { 125 };
            let p = ui.painter();
            let axes = cube::axes(&self.camera, center);
            cube::paint_axes(p, &axes, false);
            for (index, face) in faces.iter().enumerate() {
                let fill = if hovered == Some(index) {
                    Color32::from_rgb(33, 150, 243)
                } else if response.hovered() {
                    Color32::from_rgb(207, 216, 220)
                } else {
                    // Match the light idle tint with an opaque face so rear guides are masked.
                    Color32::from_rgb(225, 229, 231)
                };
                p.add(egui::Shape::convex_polygon(
                    face.points().to_vec(),
                    fill,
                    Stroke::new(0.7, Color32::from_rgba_unmultiplied(25, 118, 162, alpha)),
                ));
            }
            cube::paint_axes(p, &axes, true);
            hovered.map(|index| faces[index].normal)
        };
        if self.camera.orbit_direction()[2] < -1e-8 {
            ui.painter().text(
                center + Vec2::new(0., 70.),
                Align2::CENTER_TOP,
                "Underside",
                FontId::proportional(10.),
                Color32::from_gray(80),
            );
        }
        if let Some(direction) = hit {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if response.clicked() {
                self.camera.view_direction(direction);
            }
        }
        let home = Rect::from_center_size(center + Vec2::new(-73., -45.), Vec2::splat(30.));
        if button(
            ui,
            home,
            "home",
            Icon::Home,
            Color32::from_gray(165),
            "Reset view",
        ) {
            self.camera.view("iso");
            self.camera.fit(
                self.model.bounds,
                self.canvas_size.x as f64,
                self.canvas_size.y as f64,
            );
        }
    }
}
fn section(ui: &mut egui::Ui, rect: Rect, id: &str, kind: Icon, text: &str, open: bool) -> bool {
    let head = Rect::from_min_size(rect.min, Vec2::new(rect.width(), SECTION_HEIGHT));
    ui.painter().line_segment(
        [head.min, Pos2::new(head.max.x, head.min.y)],
        Stroke::new(1., Color32::from_gray(225)),
    );
    let response = section_header(ui, head, id, kind, text, open);
    section_title(ui, section_title_rect(head), text);
    response.clicked()
}
fn section_title_rect(head: Rect) -> Rect {
    Rect::from_min_max(head.min + Vec2::new(38., 7.), head.max - Vec2::new(34., 7.))
}
fn section_title(ui: &egui::Ui, rect: Rect, text: &str) -> f32 {
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        rect.width(),
        FontId::proportional(SECTION_TITLE_SIZE),
    );
    let right = rect.min.x + galley.size().x;
    ui.painter().with_clip_rect(rect).galley(
        Pos2::new(rect.min.x, rect.center().y - galley.size().y / 2.),
        galley,
        INK,
    );
    right
}
fn section_header(
    ui: &mut egui::Ui,
    head: Rect,
    id: &str,
    kind: Icon,
    text: &str,
    open: bool,
) -> egui::Response {
    let response = ui.interact(head, ui.scope_id().with(id), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    if response.hovered() || response.has_focus() {
        ui.painter()
            .rect_filled(head.shrink(2.), 2., Color32::from_gray(246));
    }
    icon(
        ui,
        Rect::from_min_size(head.min + Vec2::new(6., 6.), Vec2::splat(24.)),
        kind,
        INK,
    );
    icon(
        ui,
        Rect::from_min_size(
            Pos2::new(head.max.x - 30., head.min.y + 6.),
            Vec2::splat(24.),
        ),
        if open { Icon::Up } else { Icon::Down },
        INK,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
fn pill(ui: &mut egui::Ui, value: &mut bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(Vec2::new(32., 20.), Sense::click());
    let changed = r.clicked();
    if changed {
        *value = !*value;
    }
    ui.painter().rect_filled(
        Rect::from_center_size(rect.center(), Vec2::new(29., 12.)),
        6.,
        if *value {
            Color32::from_rgb(92, 151, 118)
        } else {
            Color32::from_gray(140)
        },
    );
    ui.painter().circle_filled(
        rect.center() + Vec2::new(if *value { 7. } else { -7. }, 0.),
        9.,
        if *value {
            Color32::from_rgb(128, 193, 156)
        } else {
            Color32::from_gray(230)
        },
    );
    changed
}
fn toggle(ui: &mut egui::Ui, text: &str, value: &mut bool) -> bool {
    ui.horizontal(|ui| {
        ui.label(text);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            pill(ui, value)
        })
        .inner
    })
    .inner
}

fn measured_toggle(
    ui: &mut egui::Ui,
    text: &str,
    enabled: &mut bool,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    step: f64,
    unit: &str,
) {
    ui.horizontal(|ui| {
        ui.label(text);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            pill(ui, enabled);
            ui.allocate_ui_with_layout(
                Vec2::new(84., 21.),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.scope(|ui| {
                        let visuals = ui.visuals_mut();
                        visuals.override_text_color = Some(Color32::from_gray(30));
                        visuals.widgets.inactive.weak_bg_fill = Color32::from_gray(240);
                        visuals.widgets.hovered.weak_bg_fill = Color32::WHITE;
                        visuals.widgets.active.weak_bg_fill = Color32::WHITE;
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        ui.add_sized(
                            [46., 21.],
                            egui::DragValue::new(value)
                                .range(range.clone())
                                .speed(step)
                                .max_decimals(3),
                        )
                        .on_hover_text(text);
                        ui.vertical(|ui| {
                            for direction in [1., -1.] {
                                let (rect, r) =
                                    ui.allocate_exact_size(Vec2::new(14., 10.), Sense::click());
                                ui.painter().rect_filled(
                                    rect,
                                    1.,
                                    if r.hovered() {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_gray(215)
                                    },
                                );
                                let c = rect.center();
                                ui.painter().add(egui::Shape::convex_polygon(
                                    vec![
                                        c + Vec2::new(-3., direction * 1.5),
                                        c + Vec2::new(3., direction * 1.5),
                                        c + Vec2::new(0., direction * -2.),
                                    ],
                                    TOOL_INK,
                                    Stroke::NONE,
                                ));
                                if r.clicked() {
                                    *value = ((*value + f64::from(direction) * step) * 1000.)
                                        .round()
                                        / 1000.;
                                    *value = value.clamp(*range.start(), *range.end());
                                }
                            }
                        });
                    });
                    ui.add_sized(
                        [18., 21.],
                        egui::Label::new(egui::RichText::new(unit).size(11.)),
                    );
                },
            );
        });
    });
}

fn page_editor(ctx: &egui::Context, id: egui::Id, text: &str, rows: usize) -> Option<usize> {
    if !ctx.memory(|memory| memory.has_focus(id)) {
        return None;
    }
    let (direction, extend) = ctx.input_mut(|input| {
        for (key, direction) in [(Key::PageUp, -1isize), (Key::PageDown, 1)] {
            if input.consume_key(Modifiers::SHIFT, key) {
                return Some((direction, true));
            }
            if input.consume_key(Modifiers::NONE, key) {
                return Some((direction, false));
            }
        }
        None
    })?;
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    let mut range = state.cursor.char_range().unwrap_or_default();
    let mut line = 0usize;
    let mut column = 0;
    for ch in text.chars().take(range.primary.index.into()) {
        if ch == '\n' {
            line += 1;
            column = 0;
        } else {
            column += 1;
        }
    }
    let target = line
        .saturating_add_signed(direction * rows as isize)
        .min(text.bytes().filter(|&byte| byte == b'\n').count());
    let mut cursor = 0;
    for (index, row) in text.split('\n').enumerate() {
        if index == target {
            cursor += column.min(row.trim_end_matches('\r').chars().count());
            break;
        }
        cursor += row.chars().count() + 1;
    }
    range.primary = egui::text::CCursor::new(cursor);
    if !extend {
        range.secondary = range.primary;
    }
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
    Some(target + 1)
}

fn highlight(
    text: &str,
    size: f32,
    line_height: f32,
    dark: bool,
    rows: Option<(usize, usize)>,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let foreground = if dark {
        Color32::from_rgb(248, 248, 242)
    } else {
        Color32::BLACK
    };
    let mut append = |start, end, color| {
        if start == end {
            return;
        }
        if let Some(last) = job
            .sections
            .last_mut()
            .filter(|last| last.format.color == color)
        {
            last.byte_range.end = egui::text::ByteIndex(end);
        } else {
            job.sections.push(egui::text::LayoutSection {
                leading_space: 0.,
                byte_range: egui::text::ByteIndex(start)..egui::text::ByteIndex(end),
                format: egui::TextFormat {
                    font_id: FontId::monospace(size),
                    color,
                    line_height: Some(line_height),
                    ..Default::default()
                },
            });
        }
    };
    let (start, end) = if let Some((first, last)) = rows {
        let mut breaks = text.match_indices('\n');
        let start = if first == 0 {
            0
        } else {
            breaks.nth(first - 1).map_or(text.len(), |(i, _)| i + 1)
        };
        let end = breaks
            .nth(last - first - 1)
            .map_or(text.len(), |(i, _)| i + 1);
        (start, end)
    } else {
        (0, text.len())
    };
    let mut comment = 0;
    let mut line_comment = false;
    for byte in text[..start].bytes() {
        match byte {
            b'\n' => line_comment = false,
            b'(' if !line_comment => comment += 1,
            b')' if !line_comment && comment > 0 => comment -= 1,
            b';' if comment == 0 => line_comment = true,
            _ => {}
        }
    }
    append(0, start, foreground);
    let bytes = &text.as_bytes()[..end];
    let mut i = start;
    while i < bytes.len() {
        let start = i;
        let c = bytes[i];
        let color;
        if c == b'(' || comment > 0 {
            if c == b'(' {
                comment += 1;
            }
            i += 1;
            while i < bytes.len() {
                let c = bytes[i];
                i += 1;
                if c == b'(' {
                    comment += 1;
                }
                if c == b')' {
                    comment -= 1;
                    if comment == 0 {
                        break;
                    }
                }
            }
            color = Color32::from_rgb(35, 110, 36);
        } else if c == b';' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            color = Color32::from_rgb(35, 110, 36);
        } else if (c == b'G' || c == b'M' || c == b'g' || c == b'm' || c == b'N' || c == b'n')
            && bytes.get(i + 1).is_some_and(|c| c.is_ascii_digit())
        {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            color = if c == b'N' || c == b'n' {
                Color32::from_rgb(35, 110, 36)
            } else {
                Color32::from_rgb(26, 26, 166)
            };
        } else if c.is_ascii_digit() || c == b'.' || c == b'+' || c == b'-' {
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            color = Color32::from_rgb(0, 0, 205);
        } else if c == b'#' {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit()
                    || bytes[i] == b'<'
                    || bytes[i] == b'>'
                    || bytes[i] == b'_'
                    || bytes[i].is_ascii_lowercase())
            {
                i += 1;
            }
            color = Color32::from_rgb(49, 132, 149);
        } else {
            i += 1;
            while !text.is_char_boundary(i) {
                i += 1;
            }
            color = Color32::BLACK;
        }
        let color = if dark {
            match color {
                Color32::BLACK => Color32::from_rgb(248, 248, 242),
                c if c == Color32::from_rgb(0, 0, 205) => Color32::from_rgb(174, 129, 255),
                c if c == Color32::from_rgb(26, 26, 166) => Color32::from_rgb(249, 38, 114),
                c if c == Color32::from_rgb(49, 132, 149) => Color32::from_rgb(102, 217, 239),
                _ => Color32::from_rgb(166, 180, 129),
            }
        } else {
            color
        };
        append(start, i, color);
    }
    append(end, text.len(), foreground);
    job.text = text.to_owned();
    job.wrap.max_width = f32::INFINITY;
    job
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn syntax_preserves_unicode_and_comments() {
        let source = "N10 G1 X1.5 (café (nested))\n#100=2;fin\n";
        let job = highlight(source, 12., 14., false, None);
        assert_eq!(job.text, source);
        assert!(job
            .sections
            .iter()
            .any(|s| s.format.color == Color32::from_rgb(35, 110, 36)));
    }
}
