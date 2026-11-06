// Cottleta's Settings: a player's own picture over the whole window (screen_picture.rs).
struct Params {
    // The visible part of the picture: u0, v0, du, dv.
    uv: vec4<f32>,
    // How opaque it is drawn, in x.
    alpha: vec4<f32>,
}
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var smooth_sampler: sampler;
@group(0) @binding(2) var<uniform> params: Params;

struct Out {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One triangle over the whole window: (0,0), (2,0), (0,2) in window units.
@vertex fn vs(@builtin(vertex_index) i: u32) -> Out {
    let corner = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Out;
    out.pos = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = params.uv.xy + corner * params.uv.zw;
    return out;
}

@fragment fn fs(in: Out) -> @location(0) vec4<f32> {
    let c = textureSample(picture, smooth_sampler, in.uv);
    return vec4<f32>(c.rgb, c.a * params.alpha.x);
}
