//! wgpu quad renderer: atlas-batched glyphs/fills, cached images, FIFO VSync.
//! Initialization is asynchronous; the shell runs it on a dedicated thread.

use crate::Quad;
use layout::{RasterImage, Rect, Size};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use wgpu::util::DeviceExt;

const ATLAS_SIZE: u32 = 2048;
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
    rounded: [f32; 4],
    radius: f32,
}
struct Texture {
    _texture: wgpu::Texture,
    bind: wgpu::BindGroup,
}
struct Batch {
    texture: u64,
    clip: (u32, u32, u32, u32),
    start: u32,
    end: u32,
}

pub struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    screen: wgpu::Buffer,
    atlas: Texture,
    atlas_entries: HashMap<u64, [f32; 4]>,
    atlas_x: u32,
    atlas_y: u32,
    atlas_row: u32,
    textures: HashMap<u64, Texture>,
    vertices: wgpu::Buffer,
    vertex_capacity: usize,
    pub adapter_name: String,
}
pub struct GpuWindow {
    pub gpu: Gpu,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    configured: bool,
}

fn instance() -> wgpu::Instance {
    wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::GL,
        ..Default::default()
    })
}
impl GpuWindow {
    pub async fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: Size,
    ) -> Result<Self, String> {
        let instance = instance();
        let surface = instance.create_surface(target).map_err(|e| e.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: false,
            })
            .await
            .ok_or("no compatible GPU adapter")?;
        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .or_else(|| caps.formats.first().copied())
            .ok_or("no GPU surface format")?;
        let gpu = Gpu::from_adapter(adapter, format).await?;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1.0) as u32,
            height: size.height.max(1.0) as u32,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps
                .alpha_modes
                .first()
                .copied()
                .unwrap_or(wgpu::CompositeAlphaMode::Auto),
            view_formats: Vec::new(),
        };
        // Delay the explicit-sync Wayland swapchain until softbuffer has
        // released its surface; it must not commit without an acquire fence.
        Ok(Self {
            gpu,
            surface,
            config,
            configured: false,
        })
    }
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0
            || height == 0
            || (self.configured && self.config.width == width && self.config.height == height)
        {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.gpu.device, &self.config);
        self.configured = true;
    }
    pub fn present(&mut self, quads: &[(Quad, Rect)]) -> Result<(), String> {
        if !self.configured {
            self.resize(self.config.width, self.config.height);
        }
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.surface.configure(&self.gpu.device, &self.config);
                return Ok(());
            }
            Err(wgpu::SurfaceError::Timeout) => return Ok(()),
            Err(err) => return Err(err.to_string()),
        };
        let view = frame.texture.create_view(&Default::default());
        self.gpu
            .render(&view, self.config.width, self.config.height, quads);
        frame.present();
        Ok(())
    }
}

impl Gpu {
    pub async fn offscreen() -> Result<Self, String> {
        let instance = instance();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .await
            .ok_or("no offscreen GPU adapter")?;
        Self::from_adapter(adapter, wgpu::TextureFormat::Rgba8Unorm).await
    }
    async fn from_adapter(
        adapter: wgpu::Adapter,
        format: wgpu::TextureFormat,
    ) -> Result<Self, String> {
        let adapter_name = format!(
            "{} ({:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );
        let limits = wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("browser paint"),
                    required_limits: limits,
                    memory_hints: wgpu::MemoryHints::MemoryUsage,
                    ..Default::default()
                },
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
        let screen = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("viewport"),
            contents: bytemuck::cast_slice(&[1.0f32, 1.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("versioned quad shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/quads.wgsl").into()),
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4, 4 => Float32];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("CSS quads"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        let atlas = make_texture(&device, &layout, &sampler, &screen, ATLAS_SIZE, ATLAS_SIZE);
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("quad vertices"),
            size: 4096,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut gpu = Self {
            device,
            queue,
            pipeline,
            layout,
            sampler,
            screen,
            atlas,
            atlas_entries: HashMap::new(),
            atlas_x: 4,
            atlas_y: 0,
            atlas_row: 4,
            textures: HashMap::new(),
            vertices,
            vertex_capacity: 4096,
            adapter_name,
        };
        gpu.write_atlas(0, 0, 4, 4, &[255; 64]);
        Ok(gpu)
    }
    fn write_atlas(&mut self, x: u32, y: u32, width: u32, height: u32, data: &[u8]) {
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.atlas._texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
    fn texture(&mut self, image: &Arc<RasterImage>) -> (u64, [f32; 4]) {
        if let Some(uv) = self.atlas_entries.get(&image.id) {
            return (0, *uv);
        }
        let (w, h) = (image.width + 2, image.height + 2);
        if w <= 256 && h <= 256 {
            if self.atlas_x + w > ATLAS_SIZE {
                self.atlas_x = 0;
                self.atlas_y += self.atlas_row;
                self.atlas_row = 0;
            }
            if self.atlas_y + h <= ATLAS_SIZE {
                let (x, y) = (self.atlas_x, self.atlas_y);
                let mut data = vec![0; w as usize * h as usize * 4];
                for row in 0..h {
                    for col in 0..w {
                        let sx = col.saturating_sub(1).min(image.width - 1);
                        let sy = row.saturating_sub(1).min(image.height - 1);
                        let src = ((sy * image.width + sx) * 4) as usize;
                        let dst = ((row * w + col) * 4) as usize;
                        data[dst..dst + 4].copy_from_slice(&image.rgba[src..src + 4]);
                    }
                }
                self.write_atlas(x, y, w, h, &data);
                self.atlas_x += w;
                self.atlas_row = self.atlas_row.max(h);
                let a = ATLAS_SIZE as f32;
                let uv = [
                    (x + 1) as f32 / a,
                    (y + 1) as f32 / a,
                    image.width as f32 / a,
                    image.height as f32 / a,
                ];
                self.atlas_entries.insert(image.id, uv);
                return (0, uv);
            }
        }
        if !self.textures.contains_key(&image.id) {
            let texture = make_texture(
                &self.device,
                &self.layout,
                &self.sampler,
                &self.screen,
                image.width,
                image.height,
            );
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture._texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &image.rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(image.width * 4),
                    rows_per_image: Some(image.height),
                },
                wgpu::Extent3d {
                    width: image.width,
                    height: image.height,
                    depth_or_array_layers: 1,
                },
            );
            self.textures.insert(image.id, texture);
        }
        (image.id, [0.0, 0.0, 1.0, 1.0])
    }
    pub fn render(
        &mut self,
        target: &wgpu::TextureView,
        width: u32,
        height: u32,
        quads: &[(Quad, Rect)],
    ) {
        self.queue.write_buffer(
            &self.screen,
            0,
            bytemuck::cast_slice(&[width as f32, height as f32, 0.0, 0.0]),
        );
        let mut vertices = Vec::new();
        let mut batches: Vec<Batch> = Vec::new();
        let mut used = HashSet::new();
        let canvas = Rect::new(0.0, 0.0, width as f32, height as f32);
        for (q, clip) in quads {
            let clip = clip.intersection(canvas);
            let x = (clip.x - 0.5).ceil().max(0.0) as u32;
            let y = (clip.y - 0.5).ceil().max(0.0) as u32;
            let x1 = (clip.right() - 0.5).ceil().min(width as f32) as u32;
            let y1 = (clip.bottom() - 0.5).ceil().min(height as f32) as u32;
            if x1 <= x || y1 <= y || q.rect.width <= 0.0 || q.rect.height <= 0.0 {
                continue;
            }
            let (texture, uv) = if let Some(image) = &q.image {
                self.texture(image)
            } else {
                (
                    0,
                    [1.5 / ATLAS_SIZE as f32, 1.5 / ATLAS_SIZE as f32, 0.0, 0.0],
                )
            };
            used.insert(texture);
            let color = [q.color.r, q.color.g, q.color.b, q.color.a].map(|c| c as f32 / 255.0);
            let (rounded, radius) = q
                .rounded
                .map(|(r, n)| ([r.x, r.y, r.width, r.height], n))
                .unwrap_or(([0.0; 4], 0.0));
            let start = vertices.len() as u32;
            for (px, py) in [
                (0.0, 0.0),
                (1.0, 0.0),
                (0.0, 1.0),
                (0.0, 1.0),
                (1.0, 0.0),
                (1.0, 1.0),
            ] {
                vertices.push(Vertex {
                    position: [q.rect.x + px * q.rect.width, q.rect.y + py * q.rect.height],
                    uv: [uv[0] + px * uv[2], uv[1] + py * uv[3]],
                    color,
                    rounded,
                    radius,
                });
            }
            let clip = (x, y, x1 - x, y1 - y);
            let end = vertices.len() as u32;
            if let Some(last) = batches
                .last_mut()
                .filter(|b| b.texture == texture && b.clip == clip)
            {
                last.end = end;
            } else {
                batches.push(Batch {
                    texture,
                    clip,
                    start,
                    end,
                });
            }
        }
        self.textures.retain(|id, _| used.contains(id));
        let bytes = bytemuck::cast_slice(&vertices);
        if bytes.len() > self.vertex_capacity {
            self.vertex_capacity = bytes.len().next_power_of_two();
            self.vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("grown vertices"),
                size: self.vertex_capacity as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !bytes.is_empty() {
            self.queue.write_buffer(&self.vertices, 0, bytes);
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("page composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            for b in batches {
                let texture = if b.texture == 0 {
                    Some(&self.atlas)
                } else {
                    self.textures.get(&b.texture)
                };
                if let Some(texture) = texture {
                    pass.set_bind_group(0, &texture.bind, &[]);
                    pass.set_scissor_rect(b.clip.0, b.clip.1, b.clip.2, b.clip.3);
                    pass.draw(b.start..b.end, 0..1);
                }
            }
        }
        self.queue.submit(Some(encoder.finish()));
    }
    /// Explicit offscreen parity harness; never invoked by page content.
    pub fn snapshot(
        &mut self,
        width: u32,
        height: u32,
        quads: &[(Quad, Rect)],
    ) -> Result<Vec<u8>, String> {
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err("invalid snapshot dimensions".into());
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("parity snapshot"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        self.render(
            &texture.create_view(&Default::default()),
            width,
            height,
            quads,
        );
        let pitch = (width * 4).div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("GPU readback"),
            size: u64::from(pitch) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pitch),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::Maintain::Wait);
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let mapped = buffer.slice(..).get_mapped_range();
        let mut rgba = Vec::new();
        for row in mapped.chunks_exact(pitch as usize) {
            rgba.extend_from_slice(&row[..width as usize * 4]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(rgba)
    }
}
fn make_texture(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    screen: &wgpu::Buffer,
    width: u32,
    height: u32,
) -> Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cached image"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("image binding"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: screen.as_entire_binding(),
            },
        ],
    });
    Texture {
        _texture: texture,
        bind,
    }
}
