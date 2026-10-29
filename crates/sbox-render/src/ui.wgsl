@group(0) @binding(0) var ui_tex: texture_2d<f32>;
@group(0) @binding(1) var ui_samp: sampler;

struct VsIn {
    @location(0) pos: vec2<f32>, // clip space
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_main(v: VsIn) -> VsOut {
    var o: VsOut;
    o.pos = vec4<f32>(v.pos, 0.0, 1.0);
    o.uv = v.uv;
    o.color = v.color;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(ui_tex, ui_samp, in.uv) * in.color;
}
