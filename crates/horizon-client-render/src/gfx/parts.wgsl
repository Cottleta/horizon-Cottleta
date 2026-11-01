// Model parts posed on the GPU (parts.rs): each job is one part's polygons, moved by
// its matrix into entity vertices, as `mobs::render::draw_normals` writes them, or an
// item layer's quads as `item_render::WorldSink` writes them (`ITEM`).

struct Job {
    first: u32,
    count: u32,
    dest: u32,
    flags: u32,
    // The part's matrix: x, y, z axes and translation.
    m: array<f32, 12>,
    // The normal matrix's columns.
    n: array<f32, 9>,
    uv: array<f32, 4>,
    scroll: f32,
    color: u32,
    light: u32,
    layer: u32,
}

const TRUSTED: u32 = 1u;
const ZERO_NORMAL: u32 = 2u;
const GLINT_UV: u32 = 4u;
// An item layer's quads: positions and atlas UVs as they are, the colour (ARGB) shaded
// by the normal into the vertex (`WorldSink::quad`), no normal kept.
const ITEM: u32 = 8u;
// With `ITEM`, shade by the frame's two diffuse lights.
const LIT: u32 = 16u;
// Words of an entity `Vertex`, and floats of a `PartVertex`.
const VERTEX_WORDS: u32 = 11u;
const MESH_FLOATS: u32 = 8u;
const GROUP: u32 = 32u;

@group(0) @binding(0) var<storage, read> mesh: array<f32>;
@group(0) @binding(1) var<storage, read> jobs: array<Job>;
@group(0) @binding(2) var<storage, read_write> vertices: array<u32>;
struct Params {
    count: vec4<u32>,
    light0: vec4<f32>,
    light1: vec4<f32>,
}
@group(0) @binding(3) var<uniform> params: Params;

// `(v.clamp(-1.0, 1.0) * 127.0) as i8`, as its byte.
fn pack_snorm(v: f32) -> u32 {
    return u32(i32(clamp(v, -1.0, 1.0) * 127.0)) & 0xFFu;
}

@compute @workgroup_size(32)
fn expand(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let j = group.x + group.y * groups.x;
    if j >= params.count.x {
        return;
    }
    let job = jobs[j];
    let ax = vec3<f32>(job.m[0], job.m[1], job.m[2]);
    let ay = vec3<f32>(job.m[3], job.m[4], job.m[5]);
    let az = vec3<f32>(job.m[6], job.m[7], job.m[8]);
    let at = vec3<f32>(job.m[9], job.m[10], job.m[11]);
    let nx = vec3<f32>(job.n[0], job.n[1], job.n[2]);
    let ny = vec3<f32>(job.n[3], job.n[4], job.n[5]);
    let nz = vec3<f32>(job.n[6], job.n[7], job.n[8]);
    if (job.flags & ITEM) != 0u {
        expand_item(job, lane, ax, ay, az, at, nx, ny, nz);
        return;
    }
    for (var i = lane; i < job.count; i += GROUP) {
        let b = (job.first + i) * MESH_FLOATS;
        let p = vec3<f32>(mesh[b], mesh[b + 1u], mesh[b + 2u]) / 16.0;
        let pos = ax * p.x + ay * p.y + az * p.z + at;
        let u = mesh[b + 3u];
        let v = mesh[b + 4u];
        var su = u;
        if job.scroll != 0.0 {
            su = (u + job.scroll) % 1.0;
        }
        var packed = 0u;
        if (job.flags & ZERO_NORMAL) == 0u {
            var n = nx * mesh[b + 5u] + ny * mesh[b + 6u] + nz * mesh[b + 7u];
            if (job.flags & TRUSTED) == 0u {
                // glam's normalize_or_zero.
                let len = length(n);
                if len > 0.0 && len < 3.4e38 {
                    n = n / len;
                } else {
                    n = vec3<f32>(0.0);
                }
            }
            packed = pack_snorm(n.x) | (pack_snorm(n.y) << 8u) | (pack_snorm(n.z) << 16u) | 0x81000000u;
        }
        var glint = vec2<f32>(0.0);
        if (job.flags & GLINT_UV) != 0u {
            glint = vec2<f32>(u, v);
        }
        let w = (job.dest + i) * VERTEX_WORDS;
        vertices[w] = bitcast<u32>(pos.x);
        vertices[w + 1u] = bitcast<u32>(pos.y);
        vertices[w + 2u] = bitcast<u32>(pos.z);
        vertices[w + 3u] = bitcast<u32>(job.uv[0] + (job.uv[2] - job.uv[0]) * su);
        vertices[w + 4u] = bitcast<u32>(job.uv[1] + (job.uv[3] - job.uv[1]) * v);
        vertices[w + 5u] = job.layer;
        vertices[w + 6u] = job.color;
        vertices[w + 7u] = job.light;
        vertices[w + 8u] = packed;
        vertices[w + 9u] = bitcast<u32>(glint.x);
        vertices[w + 10u] = bitcast<u32>(glint.y);
    }
}

// `ItemFeatureRenderer`'s quads into `WorldSink` vertices.
fn expand_item(
    job: Job,
    lane: u32,
    ax: vec3<f32>,
    ay: vec3<f32>,
    az: vec3<f32>,
    at: vec3<f32>,
    nx: vec3<f32>,
    ny: vec3<f32>,
    nz: vec3<f32>,
) {
    for (var i = lane; i < job.count; i += GROUP) {
        let b = (job.first + i) * MESH_FLOATS;
        let p = vec3<f32>(mesh[b], mesh[b + 1u], mesh[b + 2u]);
        let pos = ax * p.x + ay * p.y + az * p.z + at;
        var brightness = 1.0;
        if (job.flags & LIT) != 0u {
            // glam's normalize_or_zero, then `minecraft_mix_light`.
            var n = nx * mesh[b + 5u] + ny * mesh[b + 6u] + nz * mesh[b + 7u];
            let len = length(n);
            if len > 0.0 && len < 3.4e38 {
                n = n / len;
            } else {
                n = vec3<f32>(0.0);
            }
            let l = max(dot(params.light0.xyz, n), 0.0) + max(dot(params.light1.xyz, n), 0.0);
            brightness = min(l * 0.6 + 0.4, 1.0);
        }
        let a = job.color >> 24u;
        let r = u32(f32((job.color >> 16u) & 0xFFu) * brightness);
        let g = u32(f32((job.color >> 8u) & 0xFFu) * brightness);
        let bl = u32(f32(job.color & 0xFFu) * brightness);
        let w = (job.dest + i) * VERTEX_WORDS;
        vertices[w] = bitcast<u32>(pos.x);
        vertices[w + 1u] = bitcast<u32>(pos.y);
        vertices[w + 2u] = bitcast<u32>(pos.z);
        vertices[w + 3u] = bitcast<u32>(mesh[b + 3u]);
        vertices[w + 4u] = bitcast<u32>(mesh[b + 4u]);
        vertices[w + 5u] = job.layer;
        vertices[w + 6u] = (a << 24u) | (bl << 16u) | (g << 8u) | r;
        vertices[w + 7u] = job.light;
        vertices[w + 8u] = 0u;
        vertices[w + 9u] = 0u;
        vertices[w + 10u] = 0u;
    }
}
