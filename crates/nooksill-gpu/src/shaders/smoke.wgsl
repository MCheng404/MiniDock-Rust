// Smoke raster + post. Mirrors core::smoke::Fluid::raster/post.
// P: gw,gh,pw,ph,n,CELL,RES,t, base.rgb, second.rgb, has_second,
//    bar_x, bar_w, gain
@group(0) @binding(0) var<storage, read> P: array<f32>;
@group(0) @binding(1) var<storage, read> FIELDS: array<f32>; // al | ox | oy
@group(0) @binding(2) var<storage, read> NOISE: array<f32>;  // 3 layers, n*n each
@group(0) @binding(3) var OUT: texture_storage_2d<rgba8unorm, write>;

fn bilinear(base: u32, w: u32, h: u32, cx: f32, cy: f32) -> f32 {
    let ccx = clamp(cx, 0.0, f32(w) - 1.001);
    let ccy = clamp(cy, 0.0, f32(h) - 1.001);
    let x0 = u32(ccx);
    let y0 = u32(ccy);
    let j = x0 + y0 * w;
    let fx = ccx - f32(x0);
    let fy = ccy - f32(y0);
    return (FIELDS[base + j] * (1.0 - fx) + FIELDS[base + j + 1u] * fx) * (1.0 - fy)
         + (FIELDS[base + j + w] * (1.0 - fx) + FIELDS[base + j + w + 1u] * fx) * fy;
}

fn tap(k: u32, u: f32, v: f32) -> f32 {
    let n = u32(P[4]);
    let nf = f32(n);
    var fx = u - floor(u / nf) * nf;
    var fy = v - floor(v / nf) * nf;
    if (fx >= nf) { fx = fx - nf; }
    if (fy >= nf) { fy = fy - nf; }
    let x0 = u32(fx);
    let y0 = u32(fy);
    let x1 = (x0 + 1u) % n;
    let y1 = (y0 + 1u) % n;
    let ax = fx - f32(x0);
    let ay = fy - f32(y0);
    let b = k * n * n;
    let a00 = NOISE[b + y0 * n + x0];
    let a10 = NOISE[b + y0 * n + x1];
    let a01 = NOISE[b + y1 * n + x0];
    let a11 = NOISE[b + y1 * n + x1];
    return (a00 * (1.0 - ax) + a10 * ax) * (1.0 - ay)
         + (a01 * (1.0 - ax) + a11 * ax) * ay;
}

@compute @workgroup_size(64)
fn raster(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    let gw = u32(P[0]);
    let gh = u32(P[1]);
    let pw = u32(P[2]);
    let ph = u32(P[3]);
    if (i >= pw * ph) { return; }
    let pxx = i % pw;
    let py = i / pw;
    let coord = vec2<i32>(i32(pxx), i32(py));

    let cell = P[5];
    let res = P[6];
    let t = P[7];
    let cx = (f32(pxx) + 0.5) * res / cell - 0.5;
    let cy = (f32(py) + 0.5) * res / cell - 0.5;
    let a0 = bilinear(0u, gw, gh, cx, cy);
    if (a0 < 0.004) {
        textureStore(OUT, coord, vec4<f32>(0.0));
        return;
    }

    let dx = bilinear(gw * gh, gw, gh, cx, cy) * cell;
    let dy = bilinear(2u * gw * gh, gw, gh, cx, cy) * cell;
    let wx = f32(pxx) * res - dx;
    let wy = f32(py) * res - dy;

    let base = vec3<f32>(P[8], P[9], P[10]);
    let light = vec3<f32>(
        min(base.x * 1.55 + 30.0, 255.0),
        min(base.y * 1.5 + 18.0, 255.0),
        min(base.z * 1.55 + 30.0, 255.0)
    );
    let dark = base * 0.5;

    var rest = 1.0;
    var top = tap(0u, wx / 1.55 - 3.0 * t, wy / 1.55 + 6.0 * t);
    rest = rest * (1.0 - 0.95 * top);
    let p1 = tap(1u, wx / 1.08 + 4.0 * t, wy / 1.08 + 4.0 * t);
    rest = rest * (1.0 - 0.75 * p1);
    let p2 = tap(2u, wx / 2.16 - 2.0 * t, wy / 2.16 + 8.0 * t);
    rest = rest * (1.0 - 0.60 * p2);

    let dens = 1.0 - rest;
    let a = min(dens * a0 * 0.95, 0.85);
    let m = min(dens * 1.1 + top * 0.2, 1.0);
    var rgb = floor((dark + (light - dark) * m) * a + vec3<f32>(0.5));
    var alpha = floor(a * 255.0 + 0.5);

    let has_second = P[14] > 0.5;
    let gain = max(P[17], 1.0);
    if (alpha > 0.0 && (has_second || gain > 1.0)) {
        if (has_second) {
            let second = vec3<f32>(P[11], P[12], P[13]);
            let k = clamp((f32(pxx) - P[15]) / max(P[16], 1.0), 0.0, 1.0);
            if (k > 0.004) {
                let inv = 255.0 / alpha;
                let ur = rgb.x * inv;
                let ug = rgb.y * inv;
                let ub = rgb.z * inv;
                let lum1 = max(0.3 * base.x + 0.59 * base.y + 0.11 * base.z, 1.0);
                let sl = (0.3 * ur + 0.59 * ug + 0.11 * ub) / lum1;
                let af = alpha / 255.0;
                rgb = vec3<f32>(
                    (ur + (min(second.x * sl, 255.0) - ur) * k) * af,
                    (ug + (min(second.y * sl, 255.0) - ug) * k) * af,
                    (ub + (min(second.z * sl, 255.0) - ub) * k) * af
                );
            }
        }
        var na = alpha;
        if (gain > 1.0) {
            na = min(alpha * gain, 255.0);
            rgb = rgb * (na / alpha);
        }
        rgb = floor(min(rgb, vec3<f32>(na)));
        alpha = floor(na);
    }

    let out_rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(255.0)) / 255.0;
    textureStore(OUT, coord, vec4<f32>(out_rgb, clamp(alpha, 0.0, 255.0) / 255.0));
}
