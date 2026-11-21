use cnc_core::engine::{Model, Segment};
use cnc_core::geometry::{interpolate, Point};
use eframe::wgpu::{self, util::DeviceExt};
use std::sync::Arc;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SweepVertex {
    point: [f32; 3],
    owner: u32,
}

const RIM_POINTS: usize = 24;
const MAX_SWEEP_VERTICES: usize = (4 * RIM_POINTS - 4) * 3;
const PLANAR: u32 = 1 << 24;
const VOLUME: u32 = 1 << 25;
const JOINS_PREVIOUS: u32 = 1 << 26;
const JOINS_NEXT: u32 = 1 << 27;

pub(crate) fn slope(segment: &Segment) -> f64 {
    segment.end[2] - segment.start[2]
}

fn connected(before: &Segment, after: &Segment, fallback: Option<f64>) -> bool {
    !before.rapid
        && !after.rapid
        && before.end == after.start
        && before.tool == after.tool
        && before.tool_diameter.or(fallback) == after.tool_diameter.or(fallback)
}

pub(crate) fn continues_slope(before: &Segment, after: &Segment) -> bool {
    connected(before, after, None)
        && slope(before).signum() == slope(after).signum()
        && (0..2).all(|axis| {
            let a = (before.end[axis] - before.start[axis]) * slope(after);
            let b = (after.end[axis] - after.start[axis]) * slope(before);
            (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
        })
}

#[derive(Clone, Copy)]
struct SweepRange {
    step: usize,
    brush: u32,
    end: u32,
    joins: u32,
}

#[derive(Clone)]
pub struct PreparedTrail {
    vertices: Option<wgpu::Buffer>,
    ranges: Arc<[SweepRange]>,
    diameter: f64,
}

pub struct Trail {
    paint: wgpu::RenderPipeline,
    finish: wgpu::RenderPipeline,
    surface: wgpu::RenderPipeline,
    paint_bind: wgpu::BindGroup,
    finish_bind: wgpu::BindGroup,
    surface_bind: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    ranges: Arc<[SweepRange]>,
    diameter: f64,
    partial_vertices: wgpu::Buffer,
    partial: Option<(usize, f64, u32, u32)>,
}
pub fn depth() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth24PlusStencil8,
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::Always),
        stencil: Default::default(),
        bias: Default::default(),
    }
}
impl Trail {
    pub fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule, uniform: &wgpu::Buffer) -> Self {
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Uint32];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        enum Pass {
            Paint,
            Finish,
            Surface,
        }
        let pipeline = |kind| {
            let finish = matches!(kind, Pass::Finish);
            let surface = matches!(kind, Pass::Surface);
            let mut ds = depth();
            use wgpu::{CompareFunction as Compare, StencilOperation as Stencil};
            // Flat plots count brush coats. Depth-supported plots store the nearest
            // sweep separately so the resolve can compare it with the visible wire.
            let (depth_compare, compare, pass_op, read_mask, write_mask) = match kind {
                Pass::Paint => (
                    Compare::Less,
                    Compare::Equal,
                    Stencil::IncrementClamp,
                    0xfd,
                    0xff,
                ),
                Pass::Finish => (
                    Compare::Always,
                    Compare::Equal,
                    Stencil::IncrementClamp,
                    1,
                    0xff,
                ),
                Pass::Surface => (Compare::Less, Compare::Always, Stencil::Keep, 0, 0),
            };
            ds.depth_compare = Some(depth_compare);
            ds.depth_write_enabled = Some(surface);
            let face = wgpu::StencilFaceState {
                compare,
                fail_op: Stencil::Keep,
                depth_fail_op: Stencil::Keep,
                pass_op,
            };
            ds.stencil = wgpu::StencilState {
                front: face,
                back: face,
                read_mask,
                write_mask,
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(match kind {
                    Pass::Paint => "Paint brush stroke",
                    Pass::Finish => "Complete brush stroke",
                    Pass::Surface => "Depth-supported cutter sweep",
                }),
                layout: None,
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(match kind {
                        Pass::Finish => "vs_quad",
                        Pass::Paint => "vs_trail",
                        Pass::Surface => "vs_sweep",
                    }),
                    compilation_options: Default::default(),
                    buffers: if finish { &[] } else { &buffers },
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(if surface { "fs_sweep" } else { "fs_trail" }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: (!surface).then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: if finish {
                            wgpu::ColorWrites::empty()
                        } else {
                            wgpu::ColorWrites::ALL
                        },
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: Some(ds),
                multisample: wgpu::MultisampleState {
                    count: 4,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let paint = pipeline(Pass::Paint);
        let finish = pipeline(Pass::Finish);
        let surface = pipeline(Pass::Surface);
        let bind = |pipeline: &wgpu::RenderPipeline| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            })
        };
        let paint_bind = bind(&paint);
        let finish_bind = bind(&finish);
        let surface_bind = bind(&surface);
        Self {
            paint,
            finish,
            surface,
            paint_bind,
            finish_bind,
            surface_bind,
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            }),
            ranges: Arc::default(),
            diameter: 0.,
            partial: None,
            partial_vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Partial cutter sweep"),
                size: (MAX_SWEEP_VERTICES * size_of::<SweepVertex>()) as u64,
                mapped_at_creation: false,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }),
        }
    }
    pub fn diameter(&self) -> f64 {
        self.diameter
    }
    pub fn invalidate(&mut self) {
        self.diameter = 0.;
        self.partial = None;
        self.ranges = Arc::default();
    }
    pub fn prepared(&self) -> Option<PreparedTrail> {
        (self.diameter > 0.).then(|| PreparedTrail {
            vertices: (!self.ranges.is_empty()).then(|| self.vertices.clone()),
            ranges: self.ranges.clone(),
            diameter: self.diameter,
        })
    }
    pub fn load(
        &mut self,
        device: &wgpu::Device,
        model: &Model,
        diameter: f64,
        origin: [f64; 3],
    ) -> Result<(), String> {
        if self.diameter == diameter {
            return Ok(());
        }
        let data = Self::prepare(device, model, diameter, origin, None)?;
        self.install(data);
        Ok(())
    }
    pub fn prepare(
        device: &wgpu::Device,
        model: &Model,
        diameter: f64,
        origin: Point,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<PreparedTrail, String> {
        if model.segments.iter().filter(|s| !s.rapid).count() > 50000 {
            return Err(
                "Toolbit trail exceeds 50,000 cutting segments; use a shorter prefix".into(),
            );
        }
        if model.segments.iter().filter(|s| !s.rapid).any(|s| {
            s.rotation
                .iter()
                .chain(&s.start_rotation)
                .any(|v| v.abs() > 1e-9)
        }) {
            return Err("Toolbit trail currently requires a vertical cutter".into());
        }
        let mut ranges = Vec::new();
        let mut vertices = Vec::new();
        let mut brush_start = 0;
        for (index, segment) in model.segments.iter().enumerate() {
            if index % 256 == 0
                && cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            {
                return Err("Plot cancelled".into());
            }
            if segment.rapid {
                continue;
            }
            let d = segment.tool_diameter.unwrap_or(diameter);
            let radius = d / 2.;
            if !radius.is_finite() || radius <= 0. {
                continue;
            }
            let previous =
                index > 0 && connected(&model.segments[index - 1], segment, Some(diameter));
            let next = model
                .segments
                .get(index + 1)
                .is_some_and(|next| connected(segment, next, Some(diameter)));
            if !previous {
                brush_start = vertices.len() as u32;
            }
            let joins = (u32::from(previous) * JOINS_PREVIOUS) | (u32::from(next) * JOINS_NEXT);
            sweep_mesh(
                &mut vertices,
                segment.start,
                segment.end,
                radius,
                origin,
                (index as u32 + 1) | joins,
            );
            ranges.push(SweepRange {
                step: index + 1,
                brush: brush_start,
                end: vertices.len() as u32,
                joins,
            });
        }
        if cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed)) {
            return Err("Plot cancelled".into());
        }
        let vertices = (!vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Cutter sweeps"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        Ok(PreparedTrail {
            vertices,
            ranges: ranges.into(),
            diameter,
        })
    }
    pub fn install(&mut self, data: PreparedTrail) {
        if let Some(vertices) = data.vertices {
            self.vertices = vertices;
        }
        self.ranges = data.ranges;
        self.diameter = data.diameter;
        self.partial = None;
    }
    pub fn partial(
        &mut self,
        queue: &wgpu::Queue,
        request: Option<(&Segment, usize, f64)>,
        origin: Point,
    ) -> bool {
        let request = request.filter(|(segment, _, _)| !segment.rapid);
        if let Some((segment, index, fraction)) = request {
            if self.partial.is_some_and(|(old_index, old_fraction, _, _)| {
                old_index == index && old_fraction == fraction
            }) {
                return false;
            }
            let range = self
                .ranges
                .binary_search_by_key(&(index + 1), |range| range.step)
                .ok()
                .map(|position| self.ranges[position]);
            if let Some(range) = range {
                let mut vertices = Vec::with_capacity(MAX_SWEEP_VERTICES);
                let end = interpolate(segment.start, segment.end, fraction);
                sweep_mesh(
                    &mut vertices,
                    segment.start,
                    end,
                    segment.tool_diameter.unwrap_or(self.diameter) / 2.,
                    origin,
                    (index as u32 + 1) | range.joins,
                );
                queue.write_buffer(&self.partial_vertices, 0, bytemuck::cast_slice(&vertices));
                self.partial = Some((index, fraction, range.brush, vertices.len() as u32));
                return true;
            }
        }
        self.partial.take().is_some()
    }
    pub fn partial_owner(&self, segments: usize) -> u32 {
        self.partial
            .filter(|(index, _, _, _)| *index == segments)
            .map_or(0, |(index, _, _, _)| index as u32 + 1)
    }
    fn paint(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        buffer: &wgpu::Buffer,
        vertices: std::ops::Range<u32>,
    ) {
        pass.set_pipeline(&self.paint);
        pass.set_bind_group(0, &self.paint_bind, &[]);
        pass.set_stencil_reference(0);
        pass.set_vertex_buffer(0, buffer.slice(..));
        pass.draw(vertices, 0..1);
    }
    fn finish(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.finish);
        pass.set_bind_group(0, &self.finish_bind, &[]);
        pass.set_stencil_reference(1);
        pass.draw(0..3, 0..1);
    }
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, segments: usize) {
        let mut i = 0;
        let mut partial = self.partial.filter(|(index, _, _, _)| *index == segments);
        while i < self.ranges.len() && self.ranges[i].step <= segments {
            let start = self.ranges[i].brush;
            let mut end = self.ranges[i].end;
            i += 1;
            while i < self.ranges.len()
                && self.ranges[i].brush == start
                && self.ranges[i].step <= segments
            {
                end = self.ranges[i].end;
                i += 1;
            }
            self.paint(pass, &self.vertices, start..end);
            if let Some((_, _, brush, count)) = partial {
                if brush == start {
                    self.paint(pass, &self.partial_vertices, 0..count);
                    partial = None;
                }
            }
            self.finish(pass);
        }
        if let Some((_, _, _, count)) = partial {
            self.paint(pass, &self.partial_vertices, 0..count);
            self.finish(pass);
        }
    }

    pub fn draw_depth_supported(&self, pass: &mut wgpu::RenderPass<'_>, segments: usize) {
        let completed = self.ranges.partition_point(|range| range.step <= segments);
        let partial = self.partial.filter(|(index, _, _, _)| *index == segments);
        if completed == 0 && partial.is_none() {
            return;
        }
        pass.set_pipeline(&self.surface);
        pass.set_bind_group(0, &self.surface_bind, &[]);
        if let Some(completed) = completed.checked_sub(1) {
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.draw(0..self.ranges[completed].end, 0..1);
        }
        if let Some((_, _, _, count)) = partial {
            pass.set_vertex_buffer(0, self.partial_vertices.slice(..));
            pass.draw(0..count, 0..1);
        }
    }
}

fn sweep_mesh(
    vertices: &mut Vec<SweepVertex>,
    a: Point,
    b: Point,
    radius: f64,
    origin: Point,
    owner: u32,
) {
    let unit_circle = std::sync::OnceLock::get_or_init(&UNIT_CIRCLE, || {
        std::array::from_fn(|i| {
            let angle = i as f64 * std::f64::consts::TAU / RIM_POINTS as f64;
            [angle.cos(), angle.sin()]
        })
    });
    let mut triangle = |a, b, c, flags| {
        for p in [a, b, c] {
            let p: Point = p;
            vertices.push(SweepVertex {
                point: std::array::from_fn(|i| (p[i] - origin[i]) as f32),
                owner: owner | flags,
            });
        }
    };
    if (a[2] - b[2]).abs() >= 1e-6 {
        // A horizontal disc translated along the move has two horizontal caps
        // and a ruled side between corresponding points on its rims.
        let rim = |center: Point| {
            unit_circle.map(|[x, y]| [center[0] + radius * x, center[1] + radius * y, center[2]])
        };
        let [start, end] = [rim(a), rim(b)];
        for i in 1..RIM_POINTS - 1 {
            triangle(start[0], start[i], start[i + 1], VOLUME | PLANAR);
            triangle(end[0], end[i], end[i + 1], VOLUME | PLANAR);
        }
        for i in 0..RIM_POINTS {
            let j = (i + 1) % RIM_POINTS;
            triangle(start[i], start[j], end[i], VOLUME);
            triangle(end[i], start[j], end[j], VOLUME);
        }
        return;
    }
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    if length == 0. {
        let rim = unit_circle.map(|[x, y]| [a[0] + radius * x, a[1] + radius * y, a[2]]);
        for i in 1..RIM_POINTS - 1 {
            triangle(rim[0], rim[i], rim[i + 1], PLANAR);
        }
        return;
    }

    // Flat moves need only the outer capsule, with no overdraw at its caps.
    let (x, y) = (dx * radius / length, dy * radius / length);
    let rim = |center: Point, index: usize| {
        let [cos, sin] = unit_circle[index % RIM_POINTS];
        [
            center[0] + cos * x - sin * y,
            center[1] + cos * y + sin * x,
            center[2],
        ]
    };
    let left = RIM_POINTS / 4;
    let right = 3 * left;
    let (al, ar, bl, br) = (rim(a, left), rim(a, right), rim(b, left), rim(b, right));
    triangle(al, ar, bl, PLANAR);
    triangle(bl, ar, br, PLANAR);
    for (center, start) in [(a, left), (b, right)] {
        for i in start..start + RIM_POINTS / 2 {
            triangle(center, rim(center, i), rim(center, i + 1), PLANAR);
        }
    }
}

static UNIT_CIRCLE: std::sync::OnceLock<[[f64; 2]; RIM_POINTS]> = std::sync::OnceLock::new();
