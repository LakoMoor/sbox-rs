struct Post {
    // x = bloom strength (0 = off), y = bloom threshold, z = vignette intensity, w = vignette smoothness
    params: vec4<f32>,
    bloom_tint: vec4<f32>,
    // x = vignette roundness, y = aspect ratio, z = bloom radius in uv units
    extra: vec4<f32>,
};

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var hdr_samp: sampler;
@group(0) @binding(2) var<uniform> post: Post;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: VsOut;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(hdr_tex, hdr_samp, in.uv, 0.0).rgb;
    if (post.params.x > 0.0) {
        var acc = vec3<f32>(0.0);
        let r = post.extra.z;
        for (var i = 0u; i < 32u; i = i + 1u) {
            let t = (f32(i) + 0.5) / 32.0;
            let a = f32(i) * 2.39996323;
            let off = vec2<f32>(cos(a) / post.extra.y, sin(a)) * sqrt(t) * r;
            let s = textureSampleLevel(hdr_tex, hdr_samp, in.uv + off, 0.0).rgb;
            let lum = max(max(s.r, s.g), s.b);
            acc += s * max(lum - post.params.y, 0.0) / max(lum, 0.0001);
        }
        c += acc / 32.0 * post.params.x * post.bloom_tint.rgb;
    }
    var o = aces(c);
    if (post.params.z > 0.0) {
        let d = (in.uv - vec2<f32>(0.5)) * vec2<f32>(mix(1.0, post.extra.y, post.extra.x), 1.0) * 2.0;
        let v = smoothstep(1.0, 1.0 - clamp(post.params.w, 0.05, 1.0) * 0.8, length(d) / 1.4142);
        o = o * mix(1.0, v, clamp(post.params.z, 0.0, 1.0));
    }
    return vec4<f32>(o, 1.0);
}
