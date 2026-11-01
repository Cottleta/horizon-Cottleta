//! Model parts posed on the GPU. A mob's parts are the same polygons every frame, moved
//! by each part's matrix: the polygons stay in a buffer here (`PartVertex`, added once per
//! part), and the CPU sends one `PartJob` per drawn part instead of its vertices. A compute
//! pass writes the vertices `mobs::render::draw_normals` would have written into the
//! entity buffer, at the places the CPU held for them, before the world pass draws it.
//! Everything that reads the entity buffer afterwards is unchanged.
use super::upload::Uploads;

/// A part's polygon vertex in the model's own units: position, model UV and the
/// polygon's normal.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PartVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub normal: [f32; 3],
}

/// One drawn part: `count` mesh vertices from `first`, written at entity vertex `dest`
/// (`frame.entities`' index until the renderer lays the buffer out).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PartJob {
    pub first: u32,
    pub count: u32,
    pub dest: u32,
    pub flags: u32,
    /// The part's matrix: x, y and z axes, then the translation.
    pub matrix: [f32; 12],
    /// The normal matrix's columns.
    pub normal: [f32; 9],
    pub uv: [f32; 4],
    pub scroll: f32,
    pub color: u32,
    pub light: u32,
    pub layer: u32,
}

/// The normal matrix keeps lengths (no normalizing).
pub const TRUSTED: u32 = 1;
/// The normal is zero (unlit full-bright layers, the energy swirl).
pub const ZERO_NORMAL: u32 = 2;
/// The glint UV is the model UV (`ARMOR_GLINT`).
pub const GLINT_UV: u32 = 4;
/// An item layer's quads (`item_render::WorldSink`): positions and atlas UVs as they
/// are, `color` an ARGB tint shaded into the vertex, no normal kept.
pub const ITEM: u32 = 8;
/// With `ITEM`: shaded by `Frame::part_lights`.
pub const LIT: u32 = 16;
/// With `ITEM`: glinted. `uv` holds `Glint::uv`'s scale and two offsets, `scroll`'s bits
/// how many vertices after each vertex its model UV is.
pub const ITEM_GLINT: u32 = 32;
/// `Vertex::light` of a vertex whose job writes it; no lit vertex has it.
pub const PLACEHOLDER_LIGHT: u32 = u32::MAX;

/// The largest dispatch dimension every backend allows.
const MAX_GROUPS: u32 = 65535;

pub(super) struct GpuParts {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    mesh: wgpu::Buffer,
    /// Mesh vertices uploaded (a prefix of the frame's `part_mesh`).
    mesh_len: usize,
    mesh_capacity: usize,
    jobs: wgpu::Buffer,
    job_capacity: usize,
    count: wgpu::Buffer,
    /// This frame's jobs, with `dest` in the entity buffer's layout.
    laid_out: Vec<PartJob>,
}

impl GpuParts {
    /// Whether the adapter runs the compute pass (not WebGL or OpenGL ES 3.0 class).
    pub fn supported(adapter: &wgpu::Adapter) -> bool {
        adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
            && adapter.limits().max_storage_buffers_per_shader_stage >= 3
            && std::env::var_os("HORIZON_CPU_MODELS").is_none()
    }

    pub fn new(device: &wgpu::Device) -> Self {
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Model parts"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Model parts"),
            source: wgpu::ShaderSource::Wgsl(include_str!("parts.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Model parts"),
            layout: Some(
                &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("Model parts"),
                    bind_group_layouts: &[Some(&layout)],
                    ..Default::default()
                }),
            ),
            module: &module,
            entry_point: Some("expand"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            pipeline,
            layout,
            mesh: Self::buffer(device, "Model part meshes", 4096 * size_of::<PartVertex>()),
            mesh_len: 0,
            mesh_capacity: 4096,
            jobs: Self::buffer(device, "Model part jobs", 1024 * size_of::<PartJob>()),
            job_capacity: 1024,
            count: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Model part parameters"),
                size: 48,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            laid_out: Vec::new(),
        }
    }

    fn buffer(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Upload the meshes added since the last frame and this frame's jobs, `dest` moved
    /// by `place` (an `entities` vertex index to its place in the entity buffer).
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        mesh: &[PartVertex],
        jobs: &[PartJob],
        lights: [[f32; 3]; 2],
        place: impl Fn(u32) -> Option<u32>,
    ) {
        // A mesh list that shrank was started over (models reloaded): upload it again.
        if mesh.len() < self.mesh_len {
            self.mesh_len = 0;
        }
        if mesh.len() > self.mesh_capacity {
            self.mesh_capacity = mesh.len().next_power_of_two();
            self.mesh = Self::buffer(
                device,
                "Model part meshes",
                self.mesh_capacity * size_of::<PartVertex>(),
            );
            self.mesh_len = 0;
        }
        if mesh.len() > self.mesh_len {
            uploads.buffer(
                encoder,
                &self.mesh,
                (self.mesh_len * size_of::<PartVertex>()) as u64,
                bytemuck::cast_slice(&mesh[self.mesh_len..]),
            );
            self.mesh_len = mesh.len();
        }
        self.laid_out.clear();
        self.laid_out.extend(jobs.iter().filter_map(|job| {
            let dest = place(job.dest)?;
            Some(PartJob { dest, ..*job })
        }));
        if self.laid_out.is_empty() {
            return;
        }
        if self.laid_out.len() > self.job_capacity {
            self.job_capacity = self.laid_out.len().next_power_of_two();
            self.jobs = Self::buffer(
                device,
                "Model part jobs",
                self.job_capacity * size_of::<PartJob>(),
            );
        }
        uploads.buffer(encoder, &self.jobs, 0, bytemuck::cast_slice(&self.laid_out));
        let [l0, l1] = lights;
        let params: [u32; 12] = [
            self.laid_out.len() as u32,
            0,
            0,
            0,
            l0[0].to_bits(),
            l0[1].to_bits(),
            l0[2].to_bits(),
            0,
            l1[0].to_bits(),
            l1[1].to_bits(),
            l1[2].to_bits(),
            0,
        ];
        uploads.buffer(encoder, &self.count, 0, bytemuck::cast_slice(&params));
    }

    /// Write this frame's parts into `entities` (after its upload, before it is drawn).
    pub fn dispatch(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        entities: &wgpu::Buffer,
    ) {
        let jobs = self.laid_out.len() as u32;
        if jobs == 0 {
            return;
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Model parts"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.mesh.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.jobs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: entities.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.count.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Model parts"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(jobs.min(MAX_GROUPS), jobs.div_ceil(MAX_GROUPS), 1);
    }
}
