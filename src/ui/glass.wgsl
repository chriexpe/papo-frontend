struct VertexOut {
    @builtin(position) pos: vec4f,
    @location(0) uv: vec2f,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var positions = array<vec2f, 3>(
        vec2f(-1.0, -1.0),
        vec2f( 3.0, -1.0),
        vec2f(-1.0,  3.0),
    );
    let p = positions[vertex_index];
    var out: VertexOut;
    out.pos = vec4f(p, 0.0, 1.0);
    out.uv = vec2f((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    return out;
}

@group(0) @binding(0) var source_tex: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

fn blur(uv: vec2f, direction: vec2f) -> vec4f {
    let dims = vec2f(textureDimensions(source_tex));
    let step = direction / dims;
    var c = textureSample(source_tex, source_sampler, uv) * 0.2270270270;
    c += (textureSample(source_tex, source_sampler, uv + step)
        + textureSample(source_tex, source_sampler, uv - step)) * 0.1945945946;
    c += (textureSample(source_tex, source_sampler, uv + step * 2.0)
        + textureSample(source_tex, source_sampler, uv - step * 2.0)) * 0.1216216216;
    c += (textureSample(source_tex, source_sampler, uv + step * 3.0)
        + textureSample(source_tex, source_sampler, uv - step * 3.0)) * 0.0540540541;
    c += (textureSample(source_tex, source_sampler, uv + step * 4.0)
        + textureSample(source_tex, source_sampler, uv - step * 4.0)) * 0.0162162162;
    return c;
}

@fragment
fn fs_blur_h(in: VertexOut) -> @location(0) vec4f {
    return blur(in.uv, vec2f(1.0, 0.0));
}

@fragment
fn fs_blur_v(in: VertexOut) -> @location(0) vec4f {
    return blur(in.uv, vec2f(0.0, 1.0));
}

struct Params {
    size: vec2f,
    radius: f32,
    saturation: f32,
};
@group(0) @binding(2) var<uniform> params: Params;

fn rounded_box(p: vec2f, half_size: vec2f, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2f(radius);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2f(0.0))) - radius;
}

@fragment
fn fs_composite(in: VertexOut) -> @location(0) vec4f {
    let px = in.uv * params.size;
    let d = rounded_box(px - params.size * 0.5, params.size * 0.5, params.radius);
    let alpha = clamp(0.5 - d, 0.0, 1.0);
    if alpha <= 0.0 {
        discard;
    }

    var c = textureSample(source_tex, source_sampler, in.uv).rgb;
    let luma = dot(c, vec3f(0.2126, 0.7152, 0.0722));
    c = mix(vec3f(luma), c, params.saturation);
    return vec4f(c, alpha);
}
