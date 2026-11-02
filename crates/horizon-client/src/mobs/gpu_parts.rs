//! Model parts posed on the GPU (`gfx::parts`). While a frame's entity list is the
//! target (`begin`), `draw_normals` into that list holds one quad in place of a part's
//! polygons and records a job; the renderer's compute pass writes the vertices. Any other
//! list (a copy for an outline, a model drawn into a scratch list) still gets them from
//! the CPU.
//!
//! The held quad's vertices hold `PLACEHOLDER_LIGHT`, the job's number and how many quads
//! it stands for (the renderer leaves that room in the entity buffer), so jobs whose
//! quad was cut from the list (entity culling, an invisible glowing mob) are dropped
//! (`finish`); its positions are the corners of the part's box, so bounds taken from
//! the vertices (entity culling) cover the part.
use super::Part;
use crate::gfx::parts::{
    GLINT_UV, ITEM, ITEM_GLINT, LIT, PLACEHOLDER_LIGHT, PartJob, PartVertex, ZERO_NORMAL,
};
use crate::hud::Rect;
use crate::mesher::Vertex;
use horizon_assets::item_bake::BakedItemModel;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

/// Mesh vertices kept at most; past it the meshes start over (the renderer uploads
/// them again).
const MESH_LIMIT: usize = 1 << 22;

struct Mesh {
    len: usize,
    first: u32,
    min: glam::Vec3,
    max: glam::Vec3,
}

/// An item layer's quads in the mesh list: runs of quads that share a material, then
/// the same vertices' model UVs (`pos`), which the glint scrolls.
struct ItemMesh {
    first: u32,
    /// Vertices before the model UVs.
    vertices: u32,
    /// (first vertex after `first`, vertices, translucent).
    runs: Vec<(u32, u32, bool)>,
    /// The quads' one tint index; `None` when they differ or one emits light (drawn
    /// by the CPU).
    tint: Option<i32>,
    min: glam::Vec3,
    max: glam::Vec3,
}

#[derive(Default)]
struct State {
    /// By `Part::uid`: a model part keeps its uid for life and no other part gets it,
    /// where a freed part's address could come back as another (a player who left, a
    /// model reloaded) and draw with the old part's polygons.
    meshes: rustc_hash::FxHashMap<u64, Mesh>,
    /// By model and the sprite rects it was laid out with: both live until a resource
    /// reload, which empties this (`forget`).
    items: rustc_hash::FxHashMap<(*const BakedItemModel, *const HashMap<String, Rect>), ItemMesh>,
    mesh: Vec<PartVertex>,
    jobs: Vec<PartJob>,
    /// The diffuse lights of this frame's `LIT` item jobs.
    lights: Option<(glam::Vec3, glam::Vec3)>,
    /// Bumped whenever `mesh` starts over, so the renderer uploads it again
    /// (`mesh_generation`).
    generation: u32,
}
impl State {
    fn start_over(&mut self) {
        self.mesh.clear();
        self.meshes.clear();
        self.items.clear();
        self.generation = self.generation.wrapping_add(1);
    }
}

thread_local! {
    static TARGET: Cell<*const Vec<Vertex>> = const { Cell::new(std::ptr::null()) };
    static STATE: RefCell<State> = RefCell::default();
}

/// Draws into `out` hold placeholders and jobs until `end`. A frame can do this several
/// times; the meshes only start over before its first job, which would point at them.
pub fn begin(out: &Vec<Vertex>) {
    STATE.with_borrow_mut(|state| {
        if state.mesh.len() > MESH_LIMIT && state.jobs.is_empty() {
            state.start_over();
        }
    });
    TARGET.set(out);
}
/// Resources reloaded: the item models and sprite rects the item meshes were made
/// from are gone, and their addresses may come back as others'.
pub fn forget() {
    STATE.with_borrow_mut(State::start_over);
}
/// Which start of the mesh list `take_mesh` hands out: when it changes, the renderer
/// uploads the whole list again.
pub fn mesh_generation() -> u32 {
    STATE.with_borrow(|state| state.generation)
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
        let State {
            meshes, mesh, jobs, ..
        } = state;
        let known = meshes
            .get(&part.uid)
            .filter(|m| m.len == part.polygons.len());
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
                    part.uid,
                    Mesh {
                        len: part.polygons.len(),
                        first,
                        min,
                        max,
                    },
                );
                (first, min, max)
            }
        };
        let corners = moved_box(m, min, max);
        let count = part.polygons.len() * 4;
        let id = jobs.len() as u32;
        let dest = out.len() as u32;
        hold(out, count, id, overlay, corners);
        let axes = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
        let n = normal_matrix(m);
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

/// The held quad of job `id` at the end of `out`, standing for `count` vertices: the box's
/// corners first, the quads it stands for in the first vertex's `glint_uv[0]` bits
/// (`entity_batch::held_quads`).
fn hold(out: &mut Vec<Vertex>, count: usize, id: u32, layer: u32, corners: [[f32; 3]; 2]) {
    let quads = f32::from_bits((count / 4) as u32);
    let vertex = |pos| Vertex {
        pos,
        uv: [0.0; 2],
        layer,
        color: id,
        light: PLACEHOLDER_LIGHT,
        normal: [0; 4],
        glint_uv: [quads, 0.0],
    };
    let [low, high] = corners;
    out.extend_from_slice(&[vertex(low), vertex(high), vertex(low), vertex(low)]);
}

/// The normal matrix of `m`, up to its length: the inverse transpose without the
/// division by the determinant, keeping its sign. `parts.wgsl` normalizes what
/// this gives, so the directions are the inverse transpose's.
fn normal_matrix(m: glam::Mat4) -> [f32; 9] {
    let (x, y, z) = (
        m.x_axis.truncate(),
        m.y_axis.truncate(),
        m.z_axis.truncate(),
    );
    let (cx, cy, cz) = (y.cross(z), z.cross(x), x.cross(y));
    let sign = if x.dot(cx) < 0.0 { -1.0 } else { 1.0 };
    glam::Mat3::from_cols(cx * sign, cy * sign, cz * sign).to_cols_array()
}

/// The corners of the box `min`..`max` moved by `m` (Arvo: the centre moved, the
/// extents by the absolute matrix).
fn moved_box(m: glam::Mat4, min: glam::Vec3, max: glam::Vec3) -> [[f32; 3]; 2] {
    let centre = m.transform_point3((min + max) / 2.0);
    let half = (max - min) / 2.0;
    let a = glam::Mat3::from_mat4(m);
    let extent = a.x_axis.abs() * half.x + a.y_axis.abs() * half.y + a.z_axis.abs() * half.z;
    [(centre - extent).to_array(), (centre + extent).to_array()]
}

/// `item_render`'s quads of an item layer, the way `WorldSink` writes them (with `glint`,
/// glinted), posed on the GPU when `out` is the target: false when they are drawn by
/// the CPU (another list, tints that differ between quads, light emission, other lights).
#[allow(clippy::too_many_arguments)]
pub fn defer_item(
    out: &mut Vec<Vertex>,
    baked: &BakedItemModel,
    tints: &[i32],
    m: glam::Mat4,
    sprites: &HashMap<String, Rect>,
    translucent: impl Fn(&str) -> bool,
    light: u32,
    lights: Option<(glam::Vec3, glam::Vec3)>,
    glint: Option<crate::item_render::Glint>,
) -> bool {
    if !targets(out) {
        return false;
    }
    STATE.with_borrow_mut(|state| {
        let State {
            items,
            mesh,
            jobs,
            lights: frame_lights,
            ..
        } = state;
        if let (Some(l), Some(frame)) = (lights, *frame_lights)
            && l != frame
        {
            return false;
        }
        let item = items
            .entry((std::ptr::from_ref(baked), std::ptr::from_ref(sprites)))
            .or_insert_with(|| item_mesh(baked, sprites, &translucent, mesh));
        let Some(tint) = item.tint else {
            return false;
        };
        if item.runs.is_empty() {
            return true;
        }
        // `getLayerColorSafe`: an untinted quad, or a tint index past the tints, is white.
        let color = if tint >= 0 {
            tints.get(tint as usize).copied().unwrap_or(-1)
        } else {
            -1
        };
        let corners = moved_box(m, item.min, item.max);
        let axes = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
        let normal = normal_matrix(m);
        // `Glint::uv`'s scale and offsets; the model UVs follow the vertices.
        let (glinted, uv) = match glint {
            Some(g) => {
                let [o0, o1] = g.offsets();
                (ITEM_GLINT, [g.scale, o0, o1, 0.0])
            }
            None => (0, [0.0; 4]),
        };
        for &(start, count, see_through) in &item.runs {
            let layer = match (glint.is_some(), see_through) {
                (false, true) => crate::gfx::TRANSLUCENT_ITEM_OVERLAY,
                (false, false) => 0,
                (true, true) => crate::gfx::TRANSLUCENT_GLINT_ITEM_OVERLAY,
                (true, false) => 7,
            };
            let id = jobs.len() as u32;
            let dest = out.len() as u32;
            hold(out, count as usize, id, layer, corners);
            jobs.push(PartJob {
                first: item.first + start,
                count,
                dest,
                flags: ITEM | glinted | if lights.is_some() { LIT } else { 0 },
                matrix: std::array::from_fn(|i| axes[i / 3][i % 3]),
                normal,
                uv,
                // ITEM: the distance from a vertex to its model UV.
                scroll: f32::from_bits(item.vertices),
                color: color as u32,
                light,
                layer,
            });
        }
        if lights.is_some() {
            *frame_lights = lights;
        }
        true
    })
}

/// An item layer's quads into the mesh list, as `submit_quads` lays them out.
fn item_mesh(
    baked: &BakedItemModel,
    sprites: &HashMap<String, Rect>,
    translucent: &impl Fn(&str) -> bool,
    mesh: &mut Vec<PartVertex>,
) -> ItemMesh {
    let first = mesh.len() as u32;
    let mut item = ItemMesh {
        first,
        vertices: 0,
        runs: Vec::new(),
        tint: None,
        min: glam::Vec3::splat(f32::INFINITY),
        max: glam::Vec3::splat(f32::NEG_INFINITY),
    };
    let mut tint = None;
    let mut model_uv = Vec::new();
    for q in &baked.quads {
        if q.light_emission > 0 || tint.is_some_and(|t| t != q.tint_index) {
            mesh.truncate(first as usize);
            item.runs.clear();
            return item;
        }
        tint = Some(q.tint_index);
        let name = crate::item_render::sprite_name(&q.material.sprite);
        let Some(r) = sprites
            .get(name)
            .or_else(|| sprites.get("misc/missingno"))
            .or_else(|| sprites.get("horizon/missingno"))
        else {
            continue;
        };
        let see_through = translucent(name) || q.material.force_translucent;
        let at = mesh.len() as u32 - first;
        match item.runs.last_mut() {
            Some(run) if run.2 == see_through => run.1 += 4,
            _ => item.runs.push((at, 4, see_through)),
        }
        let normal = crate::item_render::direction_vector(q.facing).to_array();
        model_uv.extend(q.uv.map(|[u, v]| PartVertex {
            pos: [u, v, 0.0],
            ..PartVertex::default()
        }));
        for (p, [u, v]) in q.positions.iter().zip(q.uv) {
            let pos = glam::Vec3::from(*p);
            item.min = item.min.min(pos);
            item.max = item.max.max(pos);
            mesh.push(PartVertex {
                pos: *p,
                uv: [r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v],
                normal,
            });
        }
    }
    item.vertices = mesh.len() as u32 - first;
    mesh.extend(model_uv);
    item.tint = Some(tint.unwrap_or(-1));
    item
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

/// The frame's jobs whose placeholders are still in `entities`, into `jobs` (cleared),
/// and the diffuse lights its item jobs are shaded by; the next frame starts with none.
pub fn finish(entities: &[Vertex], jobs: &mut Vec<PartJob>) -> [[f32; 3]; 2] {
    jobs.clear();
    STATE.with_borrow_mut(|state| {
        // In place, then the lists swap: thousands of jobs in a crowd, a copy saved.
        let mut id = 0u32;
        state.jobs.retain(|job| {
            let held = entities
                .get(job.dest as usize)
                .is_some_and(|v| v.light == PLACEHOLDER_LIGHT && v.color == id)
                && job.dest as usize + 4 <= entities.len();
            id += 1;
            held
        });
        std::mem::swap(jobs, &mut state.jobs);
        let (l0, l1) = state.lights.take().unwrap_or_default();
        [l0.to_array(), l1.to_array()]
    })
}

/// Every mesh the jobs refer to, lent for the frame (`restore_mesh` gives it back).
pub fn take_mesh() -> Vec<PartVertex> {
    STATE.with_borrow_mut(|state| std::mem::take(&mut state.mesh))
}
pub fn restore_mesh(mesh: Vec<PartVertex>) {
    STATE.with_borrow_mut(|state| state.mesh = mesh);
}

#[cfg(test)]
mod normal_tests {
    /// The same directions as the inverse transpose, for a mirrored and a stretched matrix.
    #[test]
    fn normal_matrix_points_like_the_inverse_transpose() {
        let turned = glam::Mat4::from_euler(glam::EulerRot::ZYX, 0.4, -1.1, 2.3);
        for scale in [
            glam::Vec3::ONE,
            glam::Vec3::new(-1.0, 1.0, 1.0),
            glam::Vec3::new(0.5, 2.0, 1.5),
        ] {
            let m = turned * glam::Mat4::from_scale(scale);
            let exact = glam::Mat3::from_mat4(m).inverse().transpose();
            let ours = glam::Mat3::from_cols_array(&super::normal_matrix(m));
            for n in [
                glam::Vec3::X,
                glam::Vec3::Y,
                glam::Vec3::new(0.3, -0.7, 0.2),
            ] {
                let (a, b) = ((exact * n).normalize(), (ours * n).normalize());
                assert!((a - b).length() < 1e-4, "{a} against {b} at {scale}");
            }
        }
    }
}
