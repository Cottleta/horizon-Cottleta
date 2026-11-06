//! Cottleta's Settings: the player's own pictures over the whole window (a loading screen, the
//! inventory's background), drawn by the renderer from their own textures with smooth sampling,
//! between the game's HUD and the open screen (`HudBatch::blur_at`).
use std::collections::HashMap;

/// A picture to draw this frame: its name and how opaque, where it goes in the HUD's order (the
/// vertex it goes under; None: under the open screen), and the window pixels it fills (None: the
/// whole window).
#[derive(Clone, Debug, PartialEq)]
pub struct Draw {
    pub name: String,
    pub alpha: f32,
    pub at: Option<u32>,
    pub rect: Option<[f32; 4]>,
}

struct Loaded {
    texture: wgpu::Texture,
    group: wgpu::BindGroup,
    params: wgpu::Buffer,
    size: [u32; 2],
    /// What `params` holds: the visible part's UV rectangle and the alpha.
    last: Option<[f32; 5]>,
    /// When it was last drawn: a video's next frame is only wanted while it shows.
    drawn: Option<std::time::Instant>,
}

/// The pipeline and the uploaded pictures by name.
pub struct ScreenPictures {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pictures: HashMap<String, Loaded>,
}

/// The part of a `picture` sized image that covers a `target` sized window without stretching:
/// (u0, v0, du, dv), cropping the longer side evenly.
pub fn cover(picture: [u32; 2], target: [u32; 2]) -> [f32; 4] {
    let (pw, ph) = (picture[0].max(1) as f32, picture[1].max(1) as f32);
    let (tw, th) = (target[0].max(1) as f32, target[1].max(1) as f32);
    let (p, t) = (pw / ph, tw / th);
    if p > t {
        // Wider than the window: crop the sides.
        let du = t / p;
        [(1.0 - du) / 2.0, 0.0, du, 1.0]
    } else {
        let dv = p / t;
        [0.0, (1.0 - dv) / 2.0, 1.0, dv]
    }
}

impl ScreenPictures {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Screen picture"),
            source: wgpu::ShaderSource::Wgsl(include_str!("screen_picture.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Screen picture"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Screen picture"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Screen picture"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
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
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Screen picture"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
            pictures: HashMap::new(),
        }
    }

    /// Keeps `rgba` (`size`, 8 bits a channel, sRGB) as the picture `name`, replacing one.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        name: &str,
        size: [u32; 2],
        rgba: &[u8],
    ) {
        // A video's next frame: the same size, into the same texture.
        if let Some(picture) = self.pictures.get(name)
            && picture.size == size
        {
            write(queue, &picture.texture, size, rgba);
            return;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Screen picture"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write(queue, &texture, size, rgba);
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Screen picture"),
            size: 32,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let view = texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Screen picture"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        self.pictures.insert(
            name.to_owned(),
            Loaded {
                texture,
                group,
                params,
                size,
                last: None,
                drawn: None,
            },
        );
    }

    /// Forgets the picture `name` (its file went or its setting was turned off).
    pub fn remove(&mut self, name: &str) {
        self.pictures.remove(name);
    }

    pub fn has(&self, name: &str) -> bool {
        self.pictures.contains_key(name)
    }

    /// When the picture `name` was last drawn.
    pub fn drawn(&self, name: &str) -> Option<std::time::Instant> {
        self.pictures.get(name).and_then(|p| p.drawn)
    }

    /// Draws `draw`'s picture over `target` (`size` pixels), covering it. Nothing when the
    /// picture isn't uploaded.
    pub fn draw(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: [u32; 2],
        draw: &Draw,
    ) {
        let Some(picture) = self.pictures.get_mut(&draw.name) else {
            return;
        };
        picture.drawn = Some(std::time::Instant::now());
        let [x, y, w, h] = draw
            .rect
            .unwrap_or([0.0, 0.0, size[0] as f32, size[1] as f32]);
        let (x, y) = (x.max(0.0), y.max(0.0));
        let (w, h) = (w.min(size[0] as f32 - x), h.min(size[1] as f32 - y));
        if w < 1.0 || h < 1.0 {
            return;
        }
        let [u0, v0, du, dv] = cover(picture.size, [w as u32, h as u32]);
        let params = [u0, v0, du, dv, draw.alpha.clamp(0.0, 1.0)];
        if picture.last != Some(params) {
            let mut bytes = [0f32; 8];
            bytes[..5].copy_from_slice(&params);
            queue.write_buffer(&picture.params, 0, bytemuck::cast_slice(&bytes));
            picture.last = Some(params);
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Screen picture"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &picture.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// `rgba` (`size`) into all of `texture`.
fn write(queue: &wgpu::Queue, texture: &wgpu::Texture, size: [u32; 2], rgba: &[u8]) {
    queue.write_texture(
        texture.as_image_copy(),
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * size[0]),
            rows_per_image: Some(size[1]),
        },
        wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::cover;

    #[test]
    fn a_picture_covers_the_window_cropping_its_longer_side_evenly() {
        // The same shape: all of it.
        assert_eq!(cover([1920, 1080], [2560, 1440]), [0.0, 0.0, 1.0, 1.0]);
        // A square on a wide window: the middle band, top and bottom cropped.
        let [u0, v0, du, dv] = cover([1000, 1000], [2000, 1000]);
        assert_eq!((u0, du), (0.0, 1.0));
        assert!((dv - 0.5).abs() < 1e-6 && (v0 - 0.25).abs() < 1e-6);
        // A wide picture on a square window: the sides cropped.
        let [u0, _, du, _] = cover([2000, 1000], [1000, 1000]);
        assert!((du - 0.5).abs() < 1e-6 && (u0 - 0.25).abs() < 1e-6);
    }
}
