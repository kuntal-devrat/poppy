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
    // 0 = SDR, 1 = HDR PQ tone-map, 2 = HDR HLG
    tone_map: u32,
    _padding0: u32,
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

fn yuv_to_rgb(y: f32, u: f32, v: f32, matrix: u32) -> vec3<f32> {
    var r: f32;
    var g: f32;
    var b: f32;
    if (matrix == 1u) {
        // BT.601
        r = y + 1.402 * v;
        g = y - 0.344136 * u - 0.714136 * v;
        b = y + 1.772 * u;
    } else if (matrix == 2u) {
        // BT.2020 (HDR linear)
        r = y + 1.4746 * v;
        g = y - 0.16455 * u - 0.57135 * v;
        b = y + 1.8814 * u;
    } else {
        // BT.709 (HD default)
        r = y + 1.5748 * v;
        g = y - 0.187324 * u - 0.468124 * v;
        b = y + 1.8556 * u;
    }
    return vec3<f32>(r, g, b);
}

// Simple Reinhard tone-map for HDR -> SDR display (BT.2020 PQ).
fn tone_map_hdr(c: vec3<f32>, mode: u32) -> vec3<f32> {
    if (mode == 0u) {
        return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    // Reinhard + gamma 2.2 approximation for PQ/HLG.
    let mapped = c / (c + vec3<f32>(0.35));
    let graded = pow(clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2));
    return clamp(graded, vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_nv12(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_raw = textureSample(t_y, s_sampler, in.uv).r;
    let uv_raw = textureSample(t_uv, s_sampler, in.uv).rg;

    var y = y_raw;
    var u = uv_raw.r;
    var v = uv_raw.g;

    // 8-bit limited range: Y [16..235]/255, UV [16..240]/255.
    if (params.color_range == 0u) {
        y = clamp((y - 0.062745098) * 1.16438356, 0.0, 1.0);
        u = (u - 0.062745098) * 1.13777778 - 0.5;
        v = (v - 0.062745098) * 1.13777778 - 0.5;
    } else {
        u = u - 0.5;
        v = v - 0.5;
    }

    let rgb = yuv_to_rgb(y, u, v, params.color_matrix);
    let out = tone_map_hdr(rgb, params.tone_map);
    return vec4<f32>(out, 1.0);
}

@fragment
fn fs_p010(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_raw = textureSample(t_y, s_sampler, in.uv).r;
    let uv_raw = textureSample(t_uv, s_sampler, in.uv).rg;

    var y = y_raw;
    var u = uv_raw.r;
    var v = uv_raw.g;

    // 10-bit limited range in 16-bit containers: Y [64..940]/1023, UV [64..960]/1023.
    // Normalized: y_offset=64/1023≈0.06256, y_scale=1023/(940-64)≈1.1678.
    if (params.color_range == 0u) {
        y = clamp((y - 0.0625610948) * 1.16780822, 0.0, 1.0);
        u = (u - 0.0625610948) * 1.12885154 - 0.5;
        v = (v - 0.0625610948) * 1.12885154 - 0.5;
    } else {
        u = u - 0.5;
        v = v - 0.5;
    }

    let rgb = yuv_to_rgb(y, u, v, params.color_matrix);
    let out = tone_map_hdr(rgb, params.tone_map);
    return vec4<f32>(out, 1.0);
}

// Legacy entry point (NV12 path) for backward compat.
// Note: WGSL entry points cannot call each other, so logic is duplicated.
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let y_raw = textureSample(t_y, s_sampler, in.uv).r;
    let uv_raw = textureSample(t_uv, s_sampler, in.uv).rg;
    var y = y_raw;
    var u = uv_raw.r;
    var v = uv_raw.g;
    if (params.color_range == 0u) {
        y = clamp((y - 0.062745098) * 1.16438356, 0.0, 1.0);
        u = (u - 0.062745098) * 1.13777778 - 0.5;
        v = (v - 0.062745098) * 1.13777778 - 0.5;
    } else {
        u = u - 0.5;
        v = v - 0.5;
    }
    let rgb = yuv_to_rgb(y, u, v, params.color_matrix);
    let out = tone_map_hdr(rgb, params.tone_map);
    return vec4<f32>(out, 1.0);
}
