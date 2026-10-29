struct Post {
    // x = bloom strength (0 = off), y = bloom threshold, z = vignette intensity, w = vignette smoothness
    params: vec4<f32>,
    bloom_tint: vec4<f32>, // rgb = bloom tint, a = exposure multiplier
    // x = vignette roundness, y = aspect ratio, z = bloom radius in uv units, w = tonemap mode (0 ACES, 1 AgX, 2 Reinhard)
    extra: vec4<f32>,
    // x = sharpen scale (0 = off), yz = texel size in uv units
    fx: vec4<f32>,
    // x = focal distance (0 = depth of field off), y = focus range, z = max blur radius in pixels, w = blur flags (1 back, 2 front)
    dof: vec4<f32>,
    cam: vec4<f32>, // x = z near, y = z far, z = tan(fov_y / 2), w = aspect
    // xy = ssao radius (0 = off) and intensity, zw = chromatic aberration offset in pixels
    ao: vec4<f32>,
    // x = auto exposure (1 = on), y = min exposure, z = max exposure
    tm: vec4<f32>,
    // gradient fog: x = start, y = end (0 = off), z = falloff exponent, w = height
    fog: vec4<f32>,
    fog_color: vec4<f32>, // rgb, a = vertical falloff exponent
    fog_base: vec4<f32>,  // x = fog object world Z
    cam_pos: vec4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    cam_fwd: vec4<f32>,
};

@group(0) @binding(0) var hdr_tex: texture_2d<f32>;
@group(0) @binding(1) var hdr_samp: sampler;
@group(0) @binding(2) var<uniform> post: Post;
@group(0) @binding(3) var depth_tex: texture_depth_2d;

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

fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

// Polynomial AgX approximation (base look); returns linear display values.
fn agx(c: vec3<f32>) -> vec3<f32> {
    let m_in = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104));
    let m_out = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116));
    var v = m_in * max(c, vec3<f32>(1e-10));
    v = clamp(log2(v), vec3<f32>(-12.47393), vec3<f32>(4.026069));
    v = (v + 12.47393) / (4.026069 + 12.47393);
    v = agx_contrast(v);
    v = m_out * v;
    return pow(clamp(v, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
}

fn tonemap(c: vec3<f32>, mode: f32) -> vec3<f32> {
    if (mode > 1.5) {
        return c / (c + vec3<f32>(1.0));
    }
    if (mode > 0.5) {
        return agx(c);
    }
    return aces(c);
}

fn view_distance(d: f32) -> f32 {
    return post.cam.x * post.cam.y / (post.cam.y - d * (post.cam.y - post.cam.x));
}

// View-space position (x right, y up, z forward) of a pixel.
fn view_pos(px: vec2<i32>, dims: vec2<f32>) -> vec3<f32> {
    let z = view_distance(textureLoad(depth_tex, px, 0));
    let ndc = vec2<f32>((f32(px.x) + 0.5) / dims.x * 2.0 - 1.0, 1.0 - (f32(px.y) + 0.5) / dims.y * 2.0);
    return vec3<f32>(ndc.x * post.cam.w * post.cam.z * z, ndc.y * post.cam.z * z, z);
}

fn ssao(px: vec2<i32>, dims: vec2<f32>) -> f32 {
    if (textureLoad(depth_tex, px, 0) >= 0.99999) {
        return 1.0;
    }
    let p = view_pos(px, dims);
    let dx = view_pos(px + vec2<i32>(1, 0), dims) - p;
    let dy = view_pos(px + vec2<i32>(0, 1), dims) - p;
    var n = normalize(cross(dy, dx));
    if (dot(n, p) > 0.0) {
        n = -n;
    }
    let radius = post.ao.x;
    // Projected radius in pixels, capped so near surfaces stay cheap.
    let rpx = clamp(radius / p.z * dims.y / (2.0 * post.cam.z), 2.0, 64.0);
    var occ = 0.0;
    for (var i = 0u; i < 12u; i = i + 1u) {
        let t = (f32(i) + 0.5) / 12.0;
        let a = f32(i) * 2.39996323;
        let off = vec2<f32>(cos(a), sin(a)) * sqrt(t) * rpx;
        let sp = clamp(px + vec2<i32>(off), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
        let v = view_pos(sp, dims) - p;
        let l = length(v);
        if (l > 0.001 && l < radius) {
            occ += max(dot(v, n) / l - 0.15, 0.0) * (1.0 - l / radius);
        }
    }
    return 1.0 - clamp(occ / 12.0 * 2.0 * post.ao.y, 0.0, 1.0);
}

// Circle of confusion in 0..1 for a depth buffer value.
fn coc_of(d: f32) -> f32 {
    let dist = view_distance(d);
    let flags = u32(post.dof.w + 0.5);
    let range = max(post.dof.y, 1.0);
    var k = 0.0;
    if (dist > post.dof.x && (flags & 1u) != 0u) {
        k = (dist - post.dof.x) / range;
    } else if (dist < post.dof.x && (flags & 2u) != 0u) {
        k = (post.dof.x - dist) / range;
    }
    return clamp(k, 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSampleLevel(hdr_tex, hdr_samp, in.uv, 0.0).rgb;
    if (post.ao.x > 0.0) {
        c = c * ssao(vec2<i32>(in.pos.xy), vec2<f32>(textureDimensions(hdr_tex)));
    }
    if (post.fog.y > 0.0) {
        let dims = vec2<f32>(textureDimensions(hdr_tex));
        let px = vec2<i32>(in.pos.xy);
        if (textureLoad(depth_tex, px, 0) < 0.99999) {
            let v = view_pos(px, dims);
            let world = post.cam_pos.xyz + post.cam_right.xyz * v.x + post.cam_up.xyz * v.y + post.cam_fwd.xyz * v.z;
            let dist = length(v);
            let along = pow(clamp((dist - post.fog.x) / max(post.fog.y - post.fog.x, 1.0), 0.0, 1.0), post.fog.z);
            let below = pow(clamp(1.0 - (world.z - post.fog_base.x) / max(post.fog.w, 1.0), 0.0, 1.0), post.fog_color.a);
            c = mix(c, post.fog_color.rgb, along * below);
        }
    }
    if (post.dof.x > 0.0) {
        let dims = vec2<f32>(textureDimensions(hdr_tex));
        let px = vec2<i32>(in.pos.xy);
        let coc = coc_of(textureLoad(depth_tex, px, 0));
        if (coc > 0.01) {
            let radius = coc * post.dof.z * dims.y / 720.0;
            var acc = c;
            var wsum = 1.0;
            for (var i = 0u; i < 24u; i = i + 1u) {
                let t = (f32(i) + 0.5) / 24.0;
                let a = f32(i) * 2.39996323;
                let off = vec2<f32>(cos(a), sin(a)) * sqrt(t) * radius;
                let sp = clamp(px + vec2<i32>(off), vec2<i32>(0), vec2<i32>(dims) - vec2<i32>(1));
                // Weight by the sample's own blur so sharp foreground does not bleed onto the background.
                let w = max(coc_of(textureLoad(depth_tex, sp, 0)), 0.05);
                acc += textureLoad(hdr_tex, sp, 0).rgb * w;
                wsum += w;
            }
            c = acc / wsum;
        }
    }
    if (post.ao.z != 0.0 || post.ao.w != 0.0) {
        let o = post.ao.zw * post.fx.yz;
        c = vec3<f32>(textureSampleLevel(hdr_tex, hdr_samp, in.uv + o, 0.0).r, c.g, textureSampleLevel(hdr_tex, hdr_samp, in.uv - o, 0.0).b);
    }
    if (post.fx.x > 0.0) {
        let t = post.fx.yz;
        let n = textureSampleLevel(hdr_tex, hdr_samp, in.uv + vec2<f32>(0.0, -t.y), 0.0).rgb
            + textureSampleLevel(hdr_tex, hdr_samp, in.uv + vec2<f32>(0.0, t.y), 0.0).rgb
            + textureSampleLevel(hdr_tex, hdr_samp, in.uv + vec2<f32>(-t.x, 0.0), 0.0).rgb
            + textureSampleLevel(hdr_tex, hdr_samp, in.uv + vec2<f32>(t.x, 0.0), 0.0).rgb;
        c = max(c + (c - n * 0.25) * post.fx.x, vec3<f32>(0.0));
    }
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
    var exposure = post.bloom_tint.a;
    if (post.tm.x > 0.5) {
        // Geometric mean luminance over a coarse grid, mapped to mid grey. No temporal adaptation.
        var sum = 0.0;
        for (var gy = 0u; gy < 8u; gy = gy + 1u) {
            for (var gx = 0u; gx < 8u; gx = gx + 1u) {
                let s = textureSampleLevel(hdr_tex, hdr_samp, (vec2<f32>(f32(gx), f32(gy)) + 0.5) / 8.0, 0.0).rgb;
                sum += log(max(dot(s, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.0001));
            }
        }
        exposure = exposure * clamp(0.18 / exp(sum / 64.0), post.tm.y, post.tm.z);
    }
    var o = tonemap(c * exposure, post.extra.w);
    if (post.params.z > 0.0) {
        let d = (in.uv - vec2<f32>(0.5)) * vec2<f32>(mix(1.0, post.extra.y, post.extra.x), 1.0) * 2.0;
        let v = smoothstep(1.0, 1.0 - clamp(post.params.w, 0.05, 1.0) * 0.8, length(d) / 1.4142);
        o = o * mix(1.0, v, clamp(post.params.z, 0.0, 1.0));
    }
    return vec4<f32>(o, 1.0);
}
