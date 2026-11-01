//! Model parts posed on the GPU (`gfx::parts`). While a frame's entity list is the
//! target (`begin`), `draw_normals` into that list holds placeholder vertices for a part's
//! polygons and records a job; the renderer's compute pass writes the vertices. Any other
//! list (a copy for an outline, a model drawn into a scratch list) still gets them from
//! the CPU.
//!
//! A placeholder vertex holds `PLACEHOLDER_LIGHT` and its job's number, so jobs whose
//! placeholders were cut from the list (entity culling, an invisible glowing mob) are
//! dropped (`finish`); its positions are the corners of the part's box, so bounds taken
//! from the vertices (entity culling) cover the part.
use super::{Part, Polygon};
use crate::gfx::parts::{GLINT_UV, PLACEHOLDER_LIGHT, PartJob, PartVertex, ZERO_NORMAL};
use crate::mesher::Vertex;
use std::cell::{Cell, RefCell};

/// Mesh vertices kept at most; past it the meshes start over (the renderer uploads
/// them again).
const MESH_LIMIT: usize = 1 << 22;

struct Mesh {
    /// The polygons it was made from, to notice a part that reuses a freed one's address.
    polygons: *const Polygon,
    len: usize,
    first: u32,
    min: glam::Vec3,
    max: glam::Vec3,
}

#[derive(Default)]
struct State {
    meshes: rustc_hash::FxHashMap<*const Part, Mesh>,
    mesh: Vec<PartVertex>,
    jobs: Vec<PartJob>,
}

thread_local! {
    static TARGET: Cell<*const Vec<Vertex>> = const { Cell::new(std::ptr::null()) };
    static STATE: RefCell<State> = RefCell::default();
}

/// Draws into `out` hold placeholders and jobs until `end`.
pub fn begin(out: &Vec<Vertex>) {
    STATE.with_borrow_mut(|state| {
        if state.mesh.len() > MESH_LIMIT {
            state.mesh.clear();
            state.meshes.clear();
        }
    });
    TARGET.set(out);
}
pub fn end() {
    TARGET.set(std::ptr::null());
}
/// Until the guard drops, draws get their vertices from the CPU (a glowing mob's
/// outline copies them).
pub fn pause() -> Paused {
    Paused(TARGET.replace(std::ptr::null()))
}
pub struct Paused(*const Vec<Vertex>);
impl Drop for Paused {
    fn drop(&mut self) {
        TARGET.set(self.0);
    }
}

/// Whether draws into `out` are posed on the GPU (`defer`).
pub(super) fn targets(out: &Vec<Vertex>) -> bool {
    std::ptr::eq(TARGET.get(), out)
}

/// Instead of drawing `part`'s own polygons into `out` (the target), hold their places
/// and record the job; true. Its normals are `m`'s normal matrix, normalized: the CPU
/// path's exact JOML normal stack can differ from it by one step of a packed normal.
#[allow(clippy::too_many_arguments)]
pub(super) fn defer(
    out: &mut Vec<Vertex>,
    part: &Part,
    m: glam::Mat4,
    uv: [f32; 4],
    scroll: f32,
    color: u32,
    light: u32,
    overlay: u32,
    glint: bool,
) -> bool {
    if part.polygons.is_empty() {
        return true;
    }
    STATE.with_borrow_mut(|state| {
        let State { meshes, mesh, jobs } = state;
        let polygons = part.polygons.as_ptr();
        let known = meshes
            .get(&std::ptr::from_ref(part))
            .filter(|m| m.polygons == polygons && m.len == part.polygons.len());
        let (first, min, max) = match known {
            Some(m) => (m.first, m.min, m.max),
            None => {
                let first = mesh.len() as u32;
                let mut min = glam::Vec3::splat(f32::INFINITY);
                let mut max = glam::Vec3::splat(f32::NEG_INFINITY);
                for poly in &part.polygons {
                    for v in poly.vertices {
                        let p = glam::Vec3::new(v[0], v[1], v[2]) / 16.0;
                        min = min.min(p);
                        max = max.max(p);
                        mesh.push(PartVertex {
                            pos: [v[0], v[1], v[2]],
                            uv: [v[3], v[4]],
                            normal: poly.normal,
                        });
                    }
                }
                meshes.insert(
                    std::ptr::from_ref(part),
                    Mesh {
                        polygons,
                        len: part.polygons.len(),
                        first,
                        min,
                        max,
                    },
                );
                (first, min, max)
            }
        };
        // The part's box in the frame (Arvo): centre moved, extents by the absolute matrix.
        let centre = m.transform_point3((min + max) / 2.0);
        let half = (max - min) / 2.0;
        let a = glam::Mat3::from_mat4(m);
        let extent = a.x_axis.abs() * half.x + a.y_axis.abs() * half.y + a.z_axis.abs() * half.z;
        let corners = [(centre - extent).to_array(), (centre + extent).to_array()];
        let count = part.polygons.len() * 4;
        let id = jobs.len() as u32;
        let dest = out.len() as u32;
        out.extend((0..count).map(|i| Vertex {
            pos: corners[usize::from(i == 1)],
            uv: [0.0; 2],
            layer: overlay,
            color: id,
            light: PLACEHOLDER_LIGHT,
            normal: [0; 4],
            glint_uv: [0.0; 2],
        }));
        let axes = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
        let n = a.inverse().transpose().to_cols_array();
        jobs.push(PartJob {
            first,
            count: count as u32,
            dest,
            flags: if glint { GLINT_UV } else { 0 },
            matrix: std::array::from_fn(|i| axes[i / 3][i % 3]),
            normal: n,
            uv,
            scroll,
            color,
            light,
            layer: overlay,
        });
    });
    true
}

/// The jobs whose placeholders start at `start` or later draw with a zero normal (the
/// CPU path's `vertex.normal = [0; 4]` over the same vertices).
pub fn zero_normals_from(start: usize) {
    STATE.with_borrow_mut(|state| {
        for job in state.jobs.iter_mut().rev() {
            if (job.dest as usize) < start {
                break;
            }
            job.flags |= ZERO_NORMAL;
        }
    });
}

/// The frame's jobs whose placeholders are still in `entities`, into `jobs` (cleared);
/// the next frame starts with none.
pub fn finish(entities: &[Vertex], jobs: &mut Vec<PartJob>) {
    jobs.clear();
    STATE.with_borrow_mut(|state| {
        for (id, job) in state.jobs.drain(..).enumerate() {
            let held = entities
                .get(job.dest as usize)
                .is_some_and(|v| v.light == PLACEHOLDER_LIGHT && v.color == id as u32)
                && job.dest as usize + job.count as usize <= entities.len();
            if held {
                jobs.push(job);
            }
        }
    });
}

/// Every mesh the jobs refer to, lent for the frame (`restore_mesh` gives it back).
pub fn take_mesh() -> Vec<PartVertex> {
    STATE.with_borrow_mut(|state| std::mem::take(&mut state.mesh))
}
pub fn restore_mesh(mesh: Vec<PartVertex>) {
    STATE.with_borrow_mut(|state| state.mesh = mesh);
}
