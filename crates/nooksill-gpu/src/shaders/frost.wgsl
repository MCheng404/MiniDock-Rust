struct Params {
    width: u32,
    height: u32,
    radius: u32,
    taps: u32,
    corner: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var<storage, read> src: array<u32>;
@group(0) @binding(2) var<storage, read_write> mid: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> weights: array<f32>;
@group(0) @binding(4) var out_tex: texture_storage_2d<rgba8unorm, write>;

fn unpack(c: u32) -> vec3<f32> {
    return vec3<f32>(
        f32(c & 255u),
        f32((c >> 8u) & 255u),
        f32((c >> 16u) & 255u)
    );
}

fn clamp_i(v: i32, last: i32) -> i32 {
    return clamp(v, 0, last);
}

fn coverage(x: f32, y: f32) -> f32 {
    if (p.corner <= 0.0) {
        return 1.0;
    }
    let hw = f32(p.width) * 0.5;
    let hh = f32(p.height) * 0.5;
    let r = min(p.corner, min(hw, hh));
    let px = x + 0.5 - hw;
    let py = y + 0.5 - hh;
    let qx = max(abs(px) - hw + r, 0.0);
    let qy = max(abs(py) - hh + r, 0.0);
    let sdf = sqrt(qx * qx + qy * qy) - r;
    return clamp(0.5 - sdf, 0.0, 1.0);
}

@compute @workgroup_size(64)
fn h_blur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.width * p.height) {
        return;
    }
    let x = i32(i % p.width);
    let y = i32(i / p.width);
    let r = i32(p.radius);
    var acc = vec3<f32>(0.0);
    for (var k = 0u; k < p.taps; k = k + 1u) {
        let sx = clamp_i(x + i32(k) - r, i32(p.width) - 1);
        acc += unpack(src[u32(y * i32(p.width) + sx)]) * weights[k];
    }
    mid[u32(x * i32(p.height) + y)] = vec4<f32>(acc, 0.0);
}

@compute @workgroup_size(64)
fn v_blur(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= p.width * p.height) {
        return;
    }
    let x = i32(i % p.width);
    let y = i32(i / p.width);
    let r = i32(p.radius);
    var acc = vec3<f32>(0.0);
    for (var k = 0u; k < p.taps; k = k + 1u) {
        let sy = clamp_i(y + i32(k) - r, i32(p.height) - 1);
        acc += mid[u32(x * i32(p.height) + sy)].xyz * weights[k];
    }

    let rgb = floor(clamp(acc, vec3<f32>(0.0), vec3<f32>(255.0)) + vec3<f32>(0.5));
    let a = floor(coverage(f32(x), f32(y)) * 255.0 + 0.5);
    var out_rgb = rgb;
    if (a < 255.0) {
        out_rgb = floor(rgb * (a / 255.0) + vec3<f32>(0.5));
    }
    textureStore(out_tex, vec2<i32>(x, y), vec4<f32>(out_rgb / 255.0, a / 255.0));
}
