struct Globals {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,    // direction the light travels (world space)
    light_color: vec4<f32>,
    ambient: vec4<f32>,
    point_pos: array<vec4<f32>, 8>,   // xyz, radius
    point_color: array<vec4<f32>, 8>, // rgb * intensity
    counts: vec4<u32>,                // x = point light count
    light_vp: mat4x4<f32>,
    shadow: vec4<f32>,                // x = enabled, y = 1 / map size
};

struct Object {
    model: mat4x4<f32>,
    normal_m: mat4x4<f32>,
    tint: vec4<f32>,
    params: vec4<u32>, // x = bone base, y = skinned
    _pad: vec4<f32>,
    _pad2: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var<storage, read> bones: array<mat4x4<f32>>;
@group(0) @binding(2) var shadow_tex: texture_depth_2d;
@group(0) @binding(3) var shadow_samp: sampler_comparison;
@group(1) @binding(0) var<uniform> obj: Object;

struct Material {
    tint: vec4<f32>,
    uv_scale: vec4<f32>,
    pbr: vec4<f32>, // x = roughness scale, y = metalness, z = bump strength, w = flags (1 normal, 2 rma)
};
@group(2) @binding(0) var color_tex: texture_2d<f32>;
@group(2) @binding(1) var color_samp: sampler;
@group(2) @binding(2) var<uniform> mat: Material;
@group(2) @binding(3) var normal_tex: texture_2d<f32>;
@group(2) @binding(4) var rma_tex: texture_2d<f32>;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
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
    var pos = vec4<f32>(in.position, 1.0);
    var nrm = vec4<f32>(in.normal, 0.0);
    if (obj.params.y != 0u) {
        let b = obj.params.x;
        let skin = bones[b + in.joints.x] * in.weights.x
                 + bones[b + in.joints.y] * in.weights.y
                 + bones[b + in.joints.z] * in.weights.z
                 + bones[b + in.joints.w] * in.weights.w;
        pos = skin * pos;
        nrm = skin * nrm;
    }
    let wp = obj.model * pos;
    out.world_pos = wp.xyz;
    out.clip = g.view_proj * wp;
    out.normal = normalize((obj.normal_m * nrm).xyz);
    out.uv = in.uv;
    return out;
}

@vertex
fn vs_shadow(in: VsIn) -> @builtin(position) vec4<f32> {
    var pos = vec4<f32>(in.position, 1.0);
    if (obj.params.y != 0u) {
        let b = obj.params.x;
        let skin = bones[b + in.joints.x] * in.weights.x
                 + bones[b + in.joints.y] * in.weights.y
                 + bones[b + in.joints.z] * in.weights.z
                 + bones[b + in.joints.w] * in.weights.w;
        pos = skin * pos;
    }
    return g.light_vp * (obj.model * pos);
}

// 3x3 PCF directional shadow; returns 1 when fully lit.
fn shadow_factor(world_pos: vec3<f32>, n: vec3<f32>) -> f32 {
    if (g.shadow.x < 0.5) {
        return 1.0;
    }
    let offset_pos = world_pos + n * 1.5;
    let lp = g.light_vp * vec4<f32>(offset_pos, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 1.0 - (ndc.y * 0.5 + 0.5));
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0;
    }
    var sum = 0.0;
    for (var dx = -1; dx <= 1; dx = dx + 1) {
        for (var dy = -1; dy <= 1; dy = dy + 1) {
            let o = vec2<f32>(f32(dx), f32(dy)) * g.shadow.y;
            sum += textureSampleCompareLevel(shadow_tex, shadow_samp, uv + o, ndc.z);
        }
    }
    return sum / 9.0;
}

// Cotangent-frame normal mapping from screen-space derivatives (vertices carry no tangents).
fn perturb_normal(n: vec3<f32>, world_pos: vec3<f32>, uv: vec2<f32>, tn: vec3<f32>) -> vec3<f32> {
    let dp1 = dpdx(world_pos);
    let dp2 = dpdy(world_pos);
    let duv1 = dpdx(uv);
    let duv2 = dpdy(uv);
    let dp2perp = cross(dp2, n);
    let dp1perp = cross(n, dp1);
    let t = dp2perp * duv1.x + dp1perp * duv2.x;
    let b = dp2perp * duv1.y + dp1perp * duv2.y;
    let invmax = inverseSqrt(max(dot(t, t), dot(b, b)) + 1e-20);
    return normalize(t * invmax * tn.x + b * invmax * tn.y + n * tn.z);
}

fn distribution_ggx(n_dot_h: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (3.14159265 * d * d + 1e-6);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, rough: f32) -> f32 {
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let gv = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let gl = n_dot_l / (n_dot_l * (1.0 - k) + k);
    return gv * gl;
}

// Cook-Torrance specular + Lambert diffuse for one light. `radiance` is the incoming light colour.
fn shade(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, radiance: vec3<f32>, albedo: vec3<f32>, rough: f32, metal: f32) -> vec3<f32> {
    let n_dot_l = max(dot(n, l), 0.0);
    if (n_dot_l <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = normalize(v + l);
    let n_dot_v = max(dot(n, v), 1e-4);
    let n_dot_h = max(dot(n, h), 0.0);
    let f0 = mix(vec3<f32>(0.04), albedo, metal);
    let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
    let d = distribution_ggx(n_dot_h, max(rough * rough, 0.02));
    let gs = geometry_smith(n_dot_v, n_dot_l, rough);
    let spec = d * gs * f / (4.0 * n_dot_v * n_dot_l + 1e-4);
    let kd = (vec3<f32>(1.0) - f) * (1.0 - metal);
    return (kd * albedo + spec * 3.14159265) * radiance * n_dot_l;
}

// Narkowicz ACES filmic fit; scene lights are HDR (values > 1).
fn aces(x: vec3<f32>) -> vec3<f32> {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let geo_n = normalize(in.normal);
    let uv = in.uv * mat.uv_scale.xy;
    let flags = u32(mat.pbr.w + 0.5);
    var n = geo_n;
    // textureSample must stay in uniform control flow, so sample unconditionally.
    let nm = textureSample(normal_tex, color_samp, uv).xy * 2.0 - 1.0;
    let rma = textureSample(rma_tex, color_samp, uv).rgb;
    if ((flags & 1u) != 0u) {
        let xy = nm * mat.pbr.z;
        let tn = vec3<f32>(xy, sqrt(max(1.0 - dot(xy, xy), 0.0)));
        n = perturb_normal(geo_n, in.world_pos, uv, tn);
    }
    var rough = mat.pbr.x;
    var metal = mat.pbr.y;
    var ao = 1.0;
    if ((flags & 2u) != 0u) {
        rough = clamp(rma.r * mat.pbr.x, 0.04, 1.0);
        metal = clamp(rma.g, 0.0, 1.0);
        ao = rma.b;
    }
    let albedo4 = textureSample(color_tex, color_samp, uv);
    let albedo = obj.tint.rgb * mat.tint.rgb * albedo4.rgb;
    let v = normalize(g.camera_pos.xyz - in.world_pos);

    // hemisphere ambient: sky from +Z (up in s&box space)
    let hemi = mix(0.6, 1.0, n.z * 0.5 + 0.5);
    var color = g.ambient.rgb * hemi * albedo * (1.0 - metal) * ao;
    color += shade(n, v, -g.light_dir.xyz, g.light_color.rgb, albedo, rough, metal) * shadow_factor(in.world_pos, geo_n);
    for (var i = 0u; i < g.counts.x; i = i + 1u) {
        let to_l = g.point_pos[i].xyz - in.world_pos;
        let d = length(to_l);
        let att = clamp(1.0 - d / g.point_pos[i].w, 0.0, 1.0);
        color += shade(n, v, to_l / max(d, 0.001), g.point_color[i].rgb * att * att, albedo, rough, metal);
    }
    if ((flags & 4u) != 0u) {
        return vec4<f32>(albedo, obj.tint.a * albedo4.a);
    }
    return vec4<f32>(aces(color), obj.tint.a);
}
