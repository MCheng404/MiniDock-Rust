// Soft puff sprite: union of overlapping circles, smoothstep edge.
// Mirrors SmokeFx::puff_texture. P[0] = texture size.
@group(0) @binding(0) var<storage, read> P: array<f32>;
@group(0) @binding(1) var OUT: texture_storage_2d<rgba8unorm, write>;

fn circle(u: f32, v: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> f32 {
    let dx = (u - cx) / rx;
    let dy = (v - cy) / ry;
    return clamp(1.0 - sqrt(dx * dx + dy * dy), 0.0, 1.0);
}

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let s = u32(P[0]);
    if (i >= s * s) { return; }
    let x = i % s;
    let y = i / s;
    let u = (f32(x) + 0.5) / f32(s) * 2.2 - 1.1;
    let v = (f32(y) + 0.5) / f32(s) * 2.2 - 1.1;

    var best = circle(u, v, 0.0, 0.0, 0.62, 0.60);
    for (var k = 0u; k < 6u; k = k + 1u) {
        let ang = 6.2831853 * f32(k) / 6.0;
        let d = 0.62 + 0.08 * f32((k * 5u) % 4u);
        let r = 0.36 + 0.07 * f32((k * 3u) % 4u);
        best = max(best, circle(u, v, cos(ang) * d, sin(ang) * d * 0.86, r, r));
    }
    let a = best * best * (3.0 - 2.0 * best);
    let b = floor(a * 255.0) / 255.0;
    textureStore(OUT, vec2<i32>(i32(x), i32(y)), vec4<f32>(b, b, b, b));
}
