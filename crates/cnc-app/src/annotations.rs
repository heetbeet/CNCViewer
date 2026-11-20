use crate::{camera::Camera, render::Plot};
use cnc_core::{analysis, engine::Model};
use eframe::{egui, egui_wgpu, wgpu};
use serde_json::{json, Value};

pub fn labels(
    model: &Model,
    request: &Value,
    from: usize,
    to: usize,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let requests = if request == &Value::Bool(true) {
        let mut points = Vec::new();
        let start = model.moves.partition_point(|m| m.step < from);
        let end = model.moves.partition_point(|m| m.step <= to);
        let stride = (end.saturating_sub(start) / 8).max(1);
        for movement in model.moves[start.min(end)..end]
            .iter()
            .step_by(stride)
            .take(8)
        {
            points.push(json!({"step":movement.step}));
        }
        points
    } else if let Some(items) = request.as_array() {
        if items.len() > 64 {
            return Err("A view supports at most 64 coordinate labels".into());
        }
        items.clone()
    } else if request.is_null() || request == &Value::Bool(false) {
        return Ok(vec![]);
    } else {
        return Err("labels must be true or an array".into());
    };
    requests
        .into_iter()
        .enumerate()
        .map(|(index, item)| {
            let (point, step, line, source) = if !item["point"].is_null() {
                (analysis::point(&item["point"])?, None, None, None)
            } else {
                let state = analysis::select_state(model, &item)?;
                if !(from..=to).contains(&state.step) {
                    return Err("Label checkpoint is outside the displayed range".into());
                }
                (
                    state.position,
                    Some(state.step),
                    Some(state.line),
                    Some(&state.source),
                )
            };
            let id = item["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("P{}", index + 1));
            let text = item["text"].as_str().unwrap_or("");
            if id.len() + text.len() > 256 {
                return Err("Coordinate label text exceeds 256 bytes".into());
            }
            Ok(json!({"id":id,"text":text,"point":point,"step":step,"line":line,"source":source}))
        })
        .collect()
}
impl Plot {
    pub fn annotate(
        &mut self,
        camera: &Camera,
        size: [u32; 2],
        labels: &[Value],
    ) -> Result<(), Box<dyn std::error::Error>> {
        if labels.is_empty() {
            return Ok(());
        }
        let ctx = egui::Context::default();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(size[0] as f32, size[1] as f32),
                )),
                ..Default::default()
            },
            |ui| {
                for item in labels {
                    let p = analysis::point(&item["point"]).expect("validated label point");
                    let projected = camera.project(p);
                    let point = egui::pos2(
                        size[0] as f32 / 2. + projected[0] as f32,
                        size[1] as f32 / 2. + projected[1] as f32,
                    );
                    let text = format!(
                        "{} {}\nX {:.3}  Y {:.3}  Z {:.3}",
                        item["id"].as_str().unwrap_or(""),
                        item["text"].as_str().unwrap_or(""),
                        p[0],
                        p[1],
                        p[2]
                    );
                    let galley = ui.painter().layout_no_wrap(
                        text,
                        egui::FontId::monospace(12.),
                        egui::Color32::from_rgb(35, 65, 80),
                    );
                    let anchor = egui::pos2(
                        (point.x + 10.).clamp(3., (size[0] as f32 - galley.size().x - 8.).max(3.)),
                        (point.y - galley.size().y - 8.)
                            .clamp(3., (size[1] as f32 - galley.size().y - 8.).max(3.)),
                    );
                    ui.painter().line_segment(
                        [point, anchor],
                        egui::Stroke::new(1., egui::Color32::from_gray(100)),
                    );
                    ui.painter()
                        .circle_filled(point, 3., egui::Color32::from_rgb(195, 70, 65));
                    ui.painter().rect_filled(
                        egui::Rect::from_min_size(
                            anchor - egui::vec2(3., 3.),
                            galley.size() + egui::vec2(6., 6.),
                        ),
                        3.,
                        egui::Color32::from_white_alpha(235),
                    );
                    ui.painter().galley(anchor, galley, egui::Color32::BLACK);
                }
            },
        );
        let jobs = ctx.tessellate(output.shapes, 1.);
        let mut renderer = egui_wgpu::Renderer::new(
            &self.device,
            wgpu::TextureFormat::Rgba8Unorm,
            Default::default(),
        );
        for (id, delta) in &output.textures_delta.set {
            for patch in delta {
                renderer.update_texture(&self.device, &self.queue, *id, patch);
            }
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: size,
            pixels_per_point: 1.,
        };
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let commands =
            renderer.update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Coordinate annotations"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        self.queue
            .submit(commands.into_iter().chain([encoder.finish()]));
        Ok(())
    }
}
