use eframe::egui::{self, pos2, vec2, Color32, FontId, Pos2, Rect, Sense};

struct Credit {
    name: &'static str,
    url: &'static str,
    mark: &'static [u8],
}

const NC_VIEWER: Credit = Credit {
    name: "NC Viewer",
    url: "https://ncviewer.com/",
    mark: include_bytes!("../../../assets/nc-viewer-mark.png"),
};
const TOOLPATH: Credit = Credit {
    name: "Toolpath Labs",
    url: "https://toolpath.com/",
    mark: include_bytes!("../../../assets/toolpath-mark.png"),
};

impl Credit {
    fn mark_id(&self) -> egui::Id {
        egui::Id::unique(("credit-mark", self.name))
    }
}

pub(super) fn load_marks(ctx: &egui::Context) {
    for credit in [&NC_VIEWER, &TOOLPATH] {
        let mark = eframe::icon_data::from_png_bytes(credit.mark).expect("embedded credit mark");
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [mark.width as usize, mark.height as usize],
            &mark.rgba,
        );
        let texture = ctx.load_texture(credit.name, image, egui::TextureOptions::LINEAR);
        ctx.data_mut(|data| data.insert_temp(credit.mark_id(), texture));
    }
}

pub(super) fn show(ui: &egui::Ui, band: Rect, controls_right: f32) {
    const MARK_SIZE: f32 = 16.;
    const BADGE_SIZE: f32 = 24.;
    const INK: Color32 = Color32::from_rgb(54, 66, 61);
    if band.right() + 1. < ui.ctx().input(|input| input.content_rect().right()) {
        return;
    }
    let painter = ui.painter();
    let label = painter.layout_no_wrap("Inspired by".into(), FontId::proportional(12.), INK);
    let text_size = label.size();
    let width = 8. + text_size.x + 6. + 2. * BADGE_SIZE + 5.;
    let left = band.right() - 9. - width;
    if left <= controls_right {
        return;
    }
    let y = band.center().y;
    let panel = Rect::from_min_size(pos2(left, y - 13.), vec2(width, 26.));
    painter.rect_filled(panel, 5., Color32::from_rgb(236, 241, 238));
    let text_x = left + 8.;
    painter.galley(pos2(text_x, y - text_size.y / 2.), label, INK);
    let mut x = text_x + text_size.x + 6.;
    for credit in [&NC_VIEWER, &TOOLPATH] {
        let rect = Rect::from_min_size(pos2(x, y - 11.), vec2(BADGE_SIZE, 22.));
        let response = ui
            .interact(rect, ui.scope_id().with(credit.url), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(credit.name);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Link, ui.is_enabled(), credit.name)
        });
        if response.hovered() || response.has_focus() {
            painter.rect_filled(rect, 3., Color32::from_rgb(205, 231, 215));
        }
        if response.clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(credit.url));
        }
        if let Some(mark) = ui.data(|data| data.get_temp::<egui::TextureHandle>(credit.mark_id())) {
            let size = mark.size_vec2();
            let fitted = size * (MARK_SIZE / size.x.max(size.y));
            painter.image(
                mark.id(),
                Rect::from_center_size(rect.center(), fitted),
                Rect::from_min_max(Pos2::ZERO, pos2(1., 1.)),
                Color32::WHITE,
            );
        }
        x += BADGE_SIZE;
    }
}
