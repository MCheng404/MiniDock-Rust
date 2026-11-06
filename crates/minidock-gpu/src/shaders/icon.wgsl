// Rounded-corner alpha mask. Mirrors core::icon::apply_round_mask.
// P: w, h, x0, y0, x1, y1, radius
@group(0) @binding(0) var<storage, read> P: array<f32>;
@group(0) @binding(1) var<storage, read> PIX: array<u32>;
@group(0) @binding(2) var OUT: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let w = u32(P[0]);
    let h = u32(P[1]);
    if (i >= w * h) { return; }
    let x = i % w;
    let y = i / w;
    let bw = P[4] - P[2];
    let bh = P[5] - P[3];
    let r = min(P[6], min(bw, bh) / 2.0);
    let fx = f32(x) - P[2] + 0.5;
    let fy = f32(y) - P[3] + 0.5;

    var c = PIX[i];
    if (r > 0.0 && f32(x) >= P[2] && f32(x) < P[4]
        && f32(y) >= P[3] && f32(y) < P[5]) {
        var dx = 0.0;
        var dy = 0.0;
        if (fx < r) { dx = r - fx; }
        else if (fx > bw - r) { dx = fx - (bw - r); }
        if (fy < r) { dy = r - fy; }
        else if (fy > bh - r) { dy = fy - (bh - r); }
        if (dx > 0.0 && dy > 0.0 && dx * dx + dy * dy > r * r) {
            c = c & 0x00FFFFFFu; // clear alpha byte
        }
    }
    let col = vec4<f32>(
        f32(c & 255u),
        f32((c >> 8u) & 255u),
        f32((c >> 16u) & 255u),
        f32((c >> 24u) & 255u)
    ) / 255.0;
    textureStore(OUT, vec2<i32>(i32(x), i32(y)), col);
}
