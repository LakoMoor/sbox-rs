struct Sky {
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    params: vec4<f32>, // x = exposure scale
    tint: vec4<f32>,
}

@group(0) @binding(0) var<uniform> sky: Sky;
@group(0) @binding(1) var sky_tex: texture_cube<f32>;
@group(0) @binding(2) var sky_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    // fullscreen triangle
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - 1.0;
    var o: VsOut;
    o.pos = vec4<f32>(p, 1.0, 1.0);
    o.ndc = p;
    return o;
}

fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let far = sky.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - sky.camera_pos.xyz); // s&box world space, Z up
    // Source 2 cubemap faces are stored in the engine's Z-up world space, so the direction is used as-is (left/right handedness unverified)
    let d = vec3<f32>(dir.x, dir.y, dir.z);
    let c = textureSample(sky_tex, sky_samp, d).rgb * sky.params.x * sky.tint.rgb;
    return vec4<f32>(aces(c), 1.0);
}
