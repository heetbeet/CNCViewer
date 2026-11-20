use crate::camera::Camera;
use cnc_core::engine::Model;
use eframe::wgpu::{self, util::DeviceExt};
use std::{io::BufWriter, path::Path};
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Vertex {
    pub(crate) point: [f32; 3],
    pub(crate) color: u32,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draw {
    pub rapid: bool,
    pub grid: bool,
    pub trail: bool,
    pub points: bool,
    pub segments: usize,
}
pub struct PreparedPlot {
    vertices: Option<wgpu::Buffer>,
    count: u32,
    grid: wgpu::Buffer,
    grid_count: u32,
    origin: [f64; 3],
    five_axis: bool,
    trail: Option<crate::trail::PreparedTrail>,
    pub trail_error: Option<String>,
}

pub struct Plot {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    size: [u32; 2],
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    count: u32,
    grid: wgpu::Buffer,
    grid_count: u32,
    depth: wgpu::TextureView,
    trail: crate::trail::Trail,
    msaa: wgpu::TextureView,
    last_frame: Option<([f32; 12], Draw)>,
    points: wgpu::RenderPipeline,
    points_bind: wgpu::BindGroup,
    five_axis: bool,
    origin: [f64; 3],
}
impl Plot {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CNC path shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("plot.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("CNC paths"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 16,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0=>Float32x3,1=>Unorm8x4],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            depth_stencil: Some(crate::trail::depth()),
            multisample: wgpu::MultisampleState {
                count: 4,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("CNC camera"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let points = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Toolpath points"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_point"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: 32,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 16,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Unorm8x4,
                            offset: 28,
                            shader_location: 1,
                        },
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: Some(crate::trail::depth()),
            multisample: wgpu::MultisampleState {
                count: 4,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        let points_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &points.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let trail = crate::trail::Trail::new(&device, &shader, &uniform);
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Empty path"),
            size: 16,
            usage: wgpu::BufferUsages::VERTEX,
            mapped_at_creation: false,
        });
        let texture = Self::texture(&device, [1, 1]);
        let view = texture.create_view(&Default::default());
        let depth = Self::depth(&device, [1, 1]);
        let msaa = Self::msaa(&device, [1, 1]);
        Self {
            depth,
            trail,
            msaa,
            last_frame: None,
            points,
            points_bind,
            five_axis: true,
            origin: [0.; 3],
            device,
            queue,
            texture,
            view,
            size: [1, 1],
            pipeline,
            uniform,
            bind,
            grid: vertices.clone(),
            grid_count: 0,
            vertices,
            count: 0,
        }
    }
    fn depth(device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Brush stencil"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 4,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
    fn msaa(device: &wgpu::Device, size: [u32; 2]) -> wgpu::TextureView {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Antialiased plot"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 4,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }
    pub fn trail(&mut self, model: &Model, diameter: f64) -> Result<(), String> {
        let changed = self.trail.diameter() != diameter;
        self.trail
            .load(&self.device, model, diameter, self.origin)?;
        if changed {
            self.last_frame = None;
        }
        Ok(())
    }
    fn texture(device: &wgpu::Device, size: [u32; 2]) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Native CNC plot"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }
    pub fn partial_trail(&mut self, partial: Option<(&cnc_core::engine::Segment, usize, f64)>) {
        if self.trail.partial(&self.queue, partial, self.origin) {
            self.last_frame = None;
        }
    }
    pub fn set_five_axis(&mut self, model: &Model, enabled: bool) -> Result<(), String> {
        if self.five_axis != enabled {
            self.five_axis = enabled;
            self.load(model)?;
        }
        Ok(())
    }
    pub fn load(&mut self, model: &Model) -> Result<(), String> {
        self.count = 0;
        self.trail.invalidate();
        self.last_frame = None;
        let geometry = Self::prepare(&self.device, model, self.five_axis, None, None)?;
        self.install(geometry);
        Ok(())
    }
    pub fn prepare(
        device: &wgpu::Device,
        model: &Model,
        five_axis: bool,
        diameter: Option<f64>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<PreparedPlot, String> {
        let cancelled =
            || cancel.is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Relaxed));
        if cancelled() {
            return Err("Plot cancelled".into());
        }
        let bytes_per_segment = (2 * std::mem::size_of::<Vertex>()) as u64;
        if model.segments.len() as u64 > device.limits().max_buffer_size / bytes_per_segment {
            return Err(
                "Toolpath exceeds the graphics buffer limit. Plot a shorter file or prefix.".into(),
            );
        }
        let origin = model
            .segments
            .first()
            .map(|s| {
                if five_axis {
                    cnc_core::geometry::rotate_point(s.start, s.start_rotation)
                } else {
                    s.start
                }
            })
            .unwrap_or([0.; 3]);
        let extent = model
            .bounds
            .map(|(a, b)| a.into_iter().chain(b).map(f64::abs).fold(10., f64::max))
            .unwrap_or(10.);
        let half = 10f64.powf(extent.log10().ceil());
        let mut grid = Vec::new();
        for (axis, start_color, end_color) in [
            (0, [207, 72, 58, 255], [237, 139, 42, 255]),
            (1, [106, 204, 28, 255], [173, 218, 49, 255]),
            (2, [46, 105, 209, 255], [84, 165, 224, 255]),
        ] {
            let mut end = [0.; 3];
            end[axis] = half as f32;
            grid.push(Vertex {
                point: origin.map(|v| -v as f32),
                color: u32::from_le_bytes(start_color),
            });
            grid.push(Vertex {
                point: std::array::from_fn(|i| (end[i] as f64 - origin[i]) as f32),
                color: u32::from_le_bytes(end_color),
            });
        }
        for n in -10..=10 {
            let offset = half * n as f64 / 10.;
            let color = u32::from_le_bytes(if n == 0 {
                [153, 153, 153, 115]
            } else {
                [200, 200, 200, 115]
            });
            for points in [
                [[-half, offset, 0.], [half, offset, 0.]],
                [[offset, -half, 0.], [offset, half, 0.]],
            ] {
                for point in points {
                    grid.push(Vertex {
                        point: std::array::from_fn(|i| (point[i] - origin[i]) as f32),
                        color,
                    });
                }
            }
        }
        let grid_count = grid.len() as u32;
        let grid = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Grid and axes"),
            contents: bytemuck::cast_slice(&grid),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let mut vertices = Vec::with_capacity(model.segments.len() * 2);
        for (index, segment) in model.segments.iter().enumerate() {
            if index % 256 == 0 && cancelled() {
                return Err("Plot cancelled".into());
            }
            let color = u32::from_le_bytes(if segment.rapid {
                [251, 140, 0, 166]
            } else {
                [0, 0, 255, 255]
            });
            for (p, angles) in [
                (segment.start, segment.start_rotation),
                (segment.end, segment.rotation),
            ] {
                let p = if five_axis {
                    cnc_core::geometry::rotate_point(p, angles)
                } else {
                    p
                };
                vertices.push(Vertex {
                    point: std::array::from_fn(|i| (p[i] - origin[i]) as f32),
                    color,
                })
            }
        }
        if cancelled() {
            return Err("Plot cancelled".into());
        }
        let count = vertices.len() as u32;
        let vertices = (!vertices.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Native path vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let (trail, trail_error) = match diameter {
            Some(diameter) => {
                match crate::trail::Trail::prepare(device, model, diameter, origin, cancel) {
                    Ok(data) => (Some(data), None),
                    Err(error) => (None, Some(error)),
                }
            }
            None => (None, None),
        };
        Ok(PreparedPlot {
            vertices,
            count,
            grid,
            grid_count,
            origin,
            five_axis,
            trail,
            trail_error,
        })
    }
    pub fn install(&mut self, geometry: PreparedPlot) {
        if let Some(vertices) = geometry.vertices {
            self.vertices = vertices;
        }
        self.count = geometry.count;
        self.grid = geometry.grid;
        self.grid_count = geometry.grid_count;
        self.origin = geometry.origin;
        self.five_axis = geometry.five_axis;
        self.last_frame = None;
        self.trail.invalidate();
        if let Some(trail) = geometry.trail {
            self.trail.install(trail);
        }
    }
    pub fn render(&mut self, size: [u32; 2], dpi: f64, camera: &Camera, draw: Draw) -> bool {
        let Draw {
            rapid: show_rapid,
            grid: show_grid,
            trail: show_trail,
            points: show_points,
            segments,
        } = draw;
        let size = size.map(|n| n.clamp(1, 8192));
        let changed = size != self.size;
        if changed {
            self.depth = Self::depth(&self.device, size);
            self.msaa = Self::msaa(&self.device, size);
            self.texture = Self::texture(&self.device, size);
            self.view = self.texture.create_view(&Default::default());
            self.size = size;
        }
        let mut relative = camera.clone();
        relative.center = std::array::from_fn(|i| camera.center[i] - self.origin[i]);
        let uniform = relative.uniform(size[0] as f64 / dpi, size[1] as f64 / dpi, show_rapid);
        let signature = (uniform, draw);
        if !changed && self.last_frame == Some(signature) {
            return false;
        }
        self.last_frame = Some(signature);
        self.queue
            .write_buffer(&self.uniform, 0, bytemuck::cast_slice(&uniform));
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("CNC path pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.msaa,
                    depth_slice: None,
                    resolve_target: Some(&self.view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.949,
                            g: 0.949,
                            b: 0.949,
                            a: 1.,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),
                        store: wgpu::StoreOp::Discard,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind, &[]);
            if self.count != 0 {
                pass.set_vertex_buffer(0, self.grid.slice(..));
                let axes = self.grid_count.min(6);
                if show_grid {
                    pass.draw(axes..self.grid_count, 0..1);
                }
                pass.draw(0..axes, 0..1);
            }
            if show_trail {
                self.trail.draw(&mut pass, segments);
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind, &[]);
            }
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.draw(
                0..self
                    .count
                    .min(segments.saturating_mul(2).min(u32::MAX as usize) as u32),
                0..1,
            );
            if show_points {
                pass.set_pipeline(&self.points);
                pass.set_bind_group(0, &self.points_bind, &[]);
                pass.draw(
                    0..6,
                    0..self
                        .count
                        .min(segments.saturating_mul(2).min(u32::MAX as usize) as u32)
                        / 2,
                );
            }
        }
        self.queue.submit([encoder.finish()]);
        changed
    }
    pub fn png(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let [width, height] = self.size;
        let row = (width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PNG readback"),
            size: row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })?;
        receiver.recv_timeout(std::time::Duration::from_secs(30))??;
        let data = buffer.slice(..).get_mapped_range()?;
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for row_data in data.chunks_exact(row as usize) {
            pixels.extend_from_slice(&row_data[..width as usize * 4])
        }
        drop(data);
        buffer.unmap();
        crate::cli::write_artifact(path, |file| {
            let mut encoder = png::Encoder::new(BufWriter::new(file), width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header()?;
            writer.write_image_data(&pixels)?;
            Ok(writer.finish()?)
        })
    }
}
pub fn headless() -> Result<Plot, Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        ..Default::default()
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("CNC native renderer"),
        ..Default::default()
    }))?;
    Ok(Plot::new(device, queue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    #[test]
    fn empty_toolpath_has_no_grid_or_axes() {
        let mut plot = headless().unwrap();
        let camera = Camera {
            scale: 0.1,
            ..Camera::default()
        };
        let mut background_only = |source| {
            let model = cnc_core::engine::inspect(
                source,
                "grid.nc",
                &Default::default(),
                &Default::default(),
            )
            .unwrap();
            plot.load(&model).unwrap();
            plot.render(
                [128, 128],
                1.,
                &camera,
                Draw {
                    rapid: false,
                    grid: true,
                    trail: false,
                    points: false,
                    segments: 0,
                },
            );
            let path = std::env::temp_dir().join(format!(
                "cnc-grid-{}-{}.png",
                std::process::id(),
                model.segments.len()
            ));
            plot.png(&path).unwrap();
            let mut reader = png::Decoder::new(std::io::BufReader::new(File::open(&path).unwrap()))
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut pixels).unwrap();
            drop(reader);
            std::fs::remove_file(path).unwrap();
            pixels.chunks_exact(4).all(|pixel| pixel == &pixels[..4])
        };
        assert!(background_only("M30"));
        assert!(!background_only("G1 X8 F100\nM30"));
        assert!(background_only("M30"));
    }

    #[test]
    fn large_absolute_coordinates_keep_small_cut_detail() {
        let mut plot = headless().unwrap();
        let mut render = |offset: f64| {
            let text = format!(
                "G0X{offset:.0}Y{offset:.0}Z0\nG1X{:.0}F100\nM30",
                offset + 10.
            );
            let model = cnc_core::engine::inspect(
                &text,
                "large.nc",
                &Default::default(),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(model.status, "complete");
            plot.load(&model).unwrap();
            let mut camera = Camera::default();
            camera.view("top");
            camera.center = [offset + 5., offset, 0.];
            camera.scale = 0.1;
            plot.render(
                [256, 128],
                1.,
                &camera,
                Draw {
                    rapid: false,
                    grid: false,
                    trail: false,
                    points: false,
                    segments: model.segments.len(),
                },
            );
            let path = std::env::temp_dir().join(format!(
                "cnc-precision-{}-{offset:.0}.png",
                std::process::id()
            ));
            plot.png(&path).unwrap();
            let mut reader = png::Decoder::new(std::io::BufReader::new(File::open(&path).unwrap()))
                .read_info()
                .unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut pixels).unwrap();
            std::fs::remove_file(path).unwrap();
            pixels
                .chunks_exact(4)
                .map(|p| p[2] as u16 > p[0] as u16 + 20 && p[2] as u16 > p[1] as u16 + 20)
                .collect::<Vec<_>>()
        };
        let local = render(0.);
        let remote = render(100_000_000.);
        assert_eq!(
            local, remote,
            "GPU coordinates must be rebased before conversion to f32"
        );
    }
    #[test]
    fn continuous_brush_is_uniform_and_separate_coats_cap_at_two() {
        let mut plot = headless().expect("Native graphics adapter");
        let mut sample = |source: &str, offset: u32, sample: Option<(usize, f64)>| {
            let model = cnc_core::engine::inspect(
                source,
                "brush.nc",
                &Default::default(),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(model.status, "complete");
            plot.load(&model).unwrap();
            plot.trail(&model, 8.).unwrap();
            plot.partial_trail(
                sample.map(|(index, fraction)| (&model.segments[index], index, fraction)),
            );
            let mut camera = Camera::default();
            camera.view("top");
            camera.center = [10., 0., 0.];
            camera.scale = 0.1;
            plot.render(
                [256, 128],
                1.,
                &camera,
                Draw {
                    rapid: false,
                    grid: false,
                    trail: true,
                    points: false,
                    segments: sample.map_or(model.segments.len(), |(index, _)| index),
                },
            );
            let path =
                std::env::temp_dir().join(format!("cnc-brush-{}-{offset}.png", std::process::id()));
            plot.png(&path).unwrap();
            let decoder = png::Decoder::new(std::io::BufReader::new(File::open(&path).unwrap()));
            let mut reader = decoder.read_info().unwrap();
            let mut data = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut data).unwrap();
            std::fs::remove_file(path).unwrap();
            let i = (68 * 256 + 128) * 4;
            [data[i], data[i + 80 * 4]]
        };
        let single = sample("G0X0Y0Z0\nG1X20F100\nM30", 0, None);
        let joined = sample("G0X0Y0Z0\nG1X20F100\nG1X0\nM30", 1, None);
        let twice = sample("G0X0Y0Z0\nG1X20F100\nG0X0\nG1X20\nM30", 2, None);
        let third = sample(
            "G0X0Y0Z0\nG1X20F100\nG0X0\nG1X20\nG0X0\nG1X20\nM30",
            3,
            None,
        );
        let partial = sample("G0X0Y0Z0\nG1X20F100\nM30", 4, Some((0, 0.5)));
        assert_eq!(
            partial[0], single[0],
            "Partial sweep must color the reached cutter position"
        );
        assert!(partial[1] > single[1], "Future sweep must remain unpainted");
        assert_eq!(
            single, joined,
            "One continuous brush must not darken itself"
        );
        assert!(twice[0] < single[0], "A separate pass must add a coat");
        assert_eq!(twice, third, "A third pass must not darken the result");
    }
}
