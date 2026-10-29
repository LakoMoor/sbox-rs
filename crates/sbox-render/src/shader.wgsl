struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,    // direction the light travels (world space)
    light_color: vec4<f32>,
    ambient: vec4<f32>,
    point_pos: array<vec4<f32>, 8>,   // xyz, radius
    point_color: array<vec4<f32>, 8>, // rgb * intensity
    counts: vec4<u32>,                // x = point light count
};

struct Object {
    model: mat4x4<f32>,
    normal_m: mat4x4<f32>,
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<uniform> obj: Object;

struct Material {
    tint: vec4<f32>,
    uv_scale: vec4<f32>,
};
@group(2) @binding(0) var color_tex: texture_2d<f32>;
@group(2) @binding(1) var color_samp: sampler;
@group(2) @binding(2) var<uniform> mat: Material;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let wp = obj.model * vec4<f32>(in.position, 1.0);
    out.world_pos = wp.xyz;
    out.clip = g.view_proj * wp;
    out.normal = normalize((obj.normal_m * vec4<f32>(in.normal, 0.0)).xyz);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    // hemisphere ambient: sky from +Z (up in s&box space)
    let hemi = mix(0.6, 1.0, n.z * 0.5 + 0.5);
    var light = g.ambient.rgb * hemi;
    light += g.light_color.rgb * max(dot(n, -g.light_dir.xyz), 0.0);
    for (var i = 0u; i < g.counts.x; i = i + 1u) {
        let to_l = g.point_pos[i].xyz - in.world_pos;
        let d = length(to_l);
        let att = clamp(1.0 - d / g.point_pos[i].w, 0.0, 1.0);
        light += g.point_color[i].rgb * max(dot(n, to_l / max(d, 0.001)), 0.0) * att * att;
    }
    let albedo = textureSample(color_tex, color_samp, in.uv * mat.uv_scale.xy);
    return vec4<f32>(obj.tint.rgb * mat.tint.rgb * albedo.rgb * light, obj.tint.a);
}
