// Project Poppy - High Performance Video Rendering Pipeline
// NV12 and P010 (YUV 4:2:0) to sRGB Color Conversion Shader in WGSL

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

struct ColorParams {
    // 0 = BT.709, 1 = BT.601, 2 = BT.2020
    color_matrix: u32,
    // 0 = Limited Range (16..235), 1 = Full Range (0..255)
    color_range: u32,
    _padding0: u32,
    _padding1: u32,
};

@group(0) @binding(0) var t_y: texture_2d<f32>;
@group(0) @binding(1) var t_uv: texture_2d<f32>;
@group(0) @binding(2) var s_sampler: sampler;
@group(0) @binding(3) var<uniform> params: ColorParams;

@vertex
fn vs_main(@builtin(vertex_index) in_vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    // Canonical full-screen triangle covering [-1, 1] clip space
    let x = f32(i32(in_vertex_index << 1u) & 2) * 2.0 - 1.0;
    let y = f32(i32(in_vertex_index & 2u) * -2) + 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_raw = textureSample(t_y, s_sampler, in.uv).r;
    let uv_raw = textureSample(t_uv, s_sampler, in.uv).rg;

    var y = y_raw;
    var u = uv_raw.r;
    var v = uv_raw.g;

    // Range expansion if limited range
    if (params.color_range == 0u) {
        // Limited range: Y in [16..235], UV in [16..240] normalized to [0..1]
        y = clamp((y - 0.062745098) * 1.16438356, 0.0, 1.0);
        u = (u - 0.062745098) * 1.13777778 - 0.5;
        v = (v - 0.062745098) * 1.13777778 - 0.5;
    } else {
        // Full range: Y in [0..255], UV in [0..255]
        u = u - 0.5;
        v = v - 0.5;
    }

    var r: f32;
    var g: f32;
    var b: f32;

    if (params.color_matrix == 1u) {
        // BT.601
        r = y + 1.402 * v;
        g = y - 0.344136 * u - 0.714136 * v;
        b = y + 1.772 * u;
    } else if (params.color_matrix == 2u) {
        // BT.2020 (HDR)
        r = y + 1.4746 * v;
        g = y - 0.16455 * u - 0.57135 * v;
        b = y + 1.8814 * u;
    } else {
        // BT.709 (HD default)
        r = y + 1.5748 * v;
        g = y - 0.187324 * u - 0.468124 * v;
        b = y + 1.8556 * u;
    }

    return vec4<f32>(clamp(r, 0.0, 1.0), clamp(g, 0.0, 1.0), clamp(b, 0.0, 1.0), 1.0);
}
