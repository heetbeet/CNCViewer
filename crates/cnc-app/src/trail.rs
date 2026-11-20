use crate::render::Vertex;
use cnc_core::engine::{Model, Segment};
use cnc_core::geometry::{interpolate, Point};
use eframe::wgpu::{self, util::DeviceExt};

pub struct PreparedTrail {
    vertices: Option<wgpu::Buffer>,
    ranges: Vec<(usize, u32, u32)>,
    diameter: f64,
}

pub struct Trail {
    paint: wgpu::RenderPipeline,
    finish: wgpu::RenderPipeline,
    depth: wgpu::RenderPipeline,
    surface: wgpu::RenderPipeline,
    paint_bind: wgpu::BindGroup,
    finish_bind: wgpu::BindGroup,
    depth_bind: wgpu::BindGroup,
    surface_bind: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    ranges: Vec<(usize, u32, u32)>,
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
        let attributes = wgpu::vertex_attr_array![0=>Float32x3,1=>Unorm8x4];
        let buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 16,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        })];
        enum Pass {
            Paint,
            Finish,
            Depth,
            Surface,
        }
        let pipeline = |kind| {
            let finish = matches!(kind, Pass::Finish);
            let mut ds = depth();
            use wgpu::{CompareFunction as Compare, StencilOperation as Stencil};
            // Less preserves the centerlines at equal depth. The surface pass blends
            // only the nearest sweep, once per sample even at overlapping capsules.
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
                Pass::Depth => (Compare::Less, Compare::Always, Stencil::Replace, 1, 1),
                Pass::Surface => (Compare::Equal, Compare::Equal, Stencil::Zero, 1, 1),
            };
            ds.depth_compare = Some(depth_compare);
            ds.depth_write_enabled = Some(matches!(kind, Pass::Depth));
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
                    Pass::Depth => "Cutter sweep depth",
                    Pass::Surface => "Depth-supported cutter sweep",
                }),
                layout: None,
                vertex: wgpu::VertexState {
                    module: shader,
                    entry_point: Some(if finish { "vs_quad" } else { "vs" }),
                    compilation_options: Default::default(),
                    buffers: if finish { &[] } else { &buffers },
                },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some(if matches!(kind, Pass::Surface) {
                        "fs_depth_trail"
                    } else {
                        "fs_trail"
                    }),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: if finish || matches!(kind, Pass::Depth) {
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
        let depth = pipeline(Pass::Depth);
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
        let depth_bind = bind(&depth);
        let surface_bind = bind(&surface);
        Self {
            paint,
            finish,
            depth,
            surface,
            paint_bind,
            finish_bind,
            depth_bind,
            surface_bind,
            vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::VERTEX,
                mapped_at_creation: false,
            }),
            ranges: Vec::new(),
            diameter: 0.,
            partial: None,
            partial_vertices: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Partial cutter sweep"),
                size: 150 * 16,
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
        let mut previous = None;
        let mut brush_start = 0;
        let mut prior_index = 0;
        for (index, segment) in model.segments.iter().enumerate() {
            if index % 256 == 0
                && cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed))
            {
                return Err("Plot cancelled".into());
            }
            if segment.rapid {
                previous = None;
                continue;
            }
            let d = segment.tool_diameter.unwrap_or(diameter);
            let radius = d / 2.;
            if !radius.is_finite() || radius <= 0. {
                continue;
            }
            let connected = previous.is_some_and(|(end, tool, old_d)| {
                prior_index + 1 == index
                    && end == segment.start
                    && tool == segment.tool
                    && old_d == d
            });
            if !connected {
                brush_start = vertices.len() as u32;
            }
            capsule(&mut vertices, segment.start, segment.end, radius, origin);
            ranges.push((index + 1, brush_start, vertices.len() as u32));
            previous = Some((segment.end, segment.tool, d));
            prior_index = index;
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
            ranges,
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
            let brush = self
                .ranges
                .binary_search_by_key(&(index + 1), |range| range.0)
                .ok()
                .map(|position| self.ranges[position].1);
            if let Some(brush) = brush {
                let mut vertices = Vec::with_capacity(150);
                let end = interpolate(segment.start, segment.end, fraction);
                capsule(
                    &mut vertices,
                    segment.start,
                    end,
                    segment.tool_diameter.unwrap_or(self.diameter) / 2.,
                    origin,
                );
                queue.write_buffer(&self.partial_vertices, 0, bytemuck::cast_slice(&vertices));
                self.partial = Some((index, fraction, brush, vertices.len() as u32));
                return true;
            }
        }
        self.partial.take().is_some()
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
        while i < self.ranges.len() && self.ranges[i].0 <= segments {
            let start = self.ranges[i].1;
            let mut end = self.ranges[i].2;
            i += 1;
            while i < self.ranges.len() && self.ranges[i].1 == start && self.ranges[i].0 <= segments
            {
                end = self.ranges[i].2;
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
        let completed = self.ranges.partition_point(|range| range.0 <= segments);
        let partial = self.partial.filter(|(index, _, _, _)| *index == segments);
        if completed == 0 && partial.is_none() {
            return;
        }
        pass.set_stencil_reference(1);
        for (pipeline, bind) in [
            (&self.depth, &self.depth_bind),
            (&self.surface, &self.surface_bind),
        ] {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind, &[]);
            if let Some(completed) = completed.checked_sub(1) {
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.draw(0..self.ranges[completed].2, 0..1);
            }
            if let Some((_, _, _, count)) = partial {
                pass.set_vertex_buffer(0, self.partial_vertices.slice(..));
                pass.draw(0..count, 0..1);
            }
        }
    }
}

fn capsule(vertices: &mut Vec<Vertex>, a: Point, b: Point, radius: f64, origin: Point) {
    let unit_circle = std::sync::OnceLock::get_or_init(&UNIT_CIRCLE, || {
        std::array::from_fn(|i| {
            let angle = i as f64 * std::f64::consts::TAU / 24.;
            [angle.cos(), angle.sin()]
        })
    });
    let mut triangle = |a, b, c| {
        for p in [a, b, c] {
            let p: Point = p;
            vertices.push(Vertex {
                point: std::array::from_fn(|i| (p[i] - origin[i]) as f32),
                color: 0,
            });
        }
    };
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let length = dx.hypot(dy);
    if length > 0. {
        let x = -dy / length * radius;
        let y = dx / length * radius;
        let al = [a[0] + x, a[1] + y, a[2]];
        let ar = [a[0] - x, a[1] - y, a[2]];
        let bl = [b[0] + x, b[1] + y, b[2]];
        let br = [b[0] - x, b[1] - y, b[2]];
        triangle(al, ar, bl);
        triangle(bl, ar, br);
    }
    for center in [a, b] {
        let point = |[x, y]: [f64; 2]| [center[0] + radius * x, center[1] + radius * y, center[2]];
        for (i, start) in unit_circle.iter().enumerate() {
            triangle(center, point(*start), point(unit_circle[(i + 1) % 24]));
        }
    }
}

static UNIT_CIRCLE: std::sync::OnceLock<[[f64; 2]; 24]> = std::sync::OnceLock::new();
