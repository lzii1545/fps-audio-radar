//! 内嵌 SVG 路径渲染：解析用户提供的图标路径（脚步 / 子弹），供覆盖层与预览使用
//! 支持命令：M/m L/l H/h V/v C/c S/s A/a Z/z

use std::sync::OnceLock;
use tiny_skia::{Path, PathBuilder, Transform};

/// 脚步图标（奔跑的人，脚趾朝上 = 0°）
pub const FOOTSTEP_D: &str = "M170.666667 768h234.666666v53.333333a117.333333 117.333333 0 0 1-234.666666 0V768zM341.333333 261.12c85.333333 0 128 122.88 128 208.213333 0 42.666667-21.333333 85.333333-42.666666 149.333334L405.333333 682.666667H170.666667c0-42.666667-21.333333-106.666667-21.333334-213.333334S234.581333 261.12 341.333333 261.12z m514.304 340.394667l-9.258666 52.522666a117.333333 117.333333 0 0 1-231.082667-40.746666l9.216-52.48 231.125333 40.704zM775.594667 72.746667c105.130667 18.517333 171.434667 133.333333 152.917333 238.378666-18.517333 105.045333-50.645333 164.352-58.026667 206.378667l-231.125333-40.746667-9.898667-66.730666c-9.898667-66.730667-23.466667-112.469333-16.085333-154.453334 14.805333-84.053333 78.165333-197.632 162.218667-182.826666z";

/// 子弹图标（弹头朝右上 ≈ 45°）
pub const BULLET_D: &str = "M596.848 273.227c-17.195-17.196-45.934-17.761-63.855-1.291l-322.149 295.387c-17.881 16.47-18.526 43.997-1.291 61.194l203.113 203.273c17.157 17.197 44.642 16.551 61.154-1.453l295.463-322.186c16.47-17.923 15.865-46.663-1.331-63.776l-171.103-171.147z M155.667 649.907c-17.237-17.195-45.288-17.195-62.565 0-17.196 17.357-17.196 45.289 0 62.567l237.865 237.824c8.598 8.557 19.94 12.916 31.242 12.916 11.342 0 22.645-4.359 31.283-12.916 17.235-17.276 17.235-45.288 0-62.565l-237.826-237.826z M935.344 67.753c-67.327-1.977-208.847 8.638-301.079 116.834-1.574 3.149-16.67 33.746 6.661 57.238l160.448 161.296c6.096 5.812 25.995 20.667 56.955 3.149 42.866-41.010 117.218-136.351 116.29-301.542-0.89-30.717-28.014-36.167-39.274-36.974z";

/// 图标自然朝向（度，0 = 朝上，顺时针）：脚步朝上，子弹弹头朝右上 45°
pub const FOOTSTEP_NATURAL: f32 = 0.0;
pub const BULLET_NATURAL: f32 = 45.0;

fn parse_number(b: &[u8], i: &mut usize) -> Option<f32> {
    let start = *i;
    let n = b.len();
    if *i < n && (b[*i] == b'+' || b[*i] == b'-') {
        *i += 1;
    }
    let mut digits = 0;
    while *i < n && b[*i].is_ascii_digit() {
        *i += 1;
        digits += 1;
    }
    if *i < n && b[*i] == b'.' {
        *i += 1;
        while *i < n && b[*i].is_ascii_digit() {
            *i += 1;
            digits += 1;
        }
    }
    if *i < n && (b[*i] == b'e' || b[*i] == b'E') {
        *i += 1;
        if *i < n && (b[*i] == b'+' || b[*i] == b'-') {
            *i += 1;
        }
        while *i < n && b[*i].is_ascii_digit() {
            *i += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    std::str::from_utf8(&b[start..*i]).ok()?.parse().ok()
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && (b[*i].is_ascii_whitespace() || b[*i] == b',') {
        *i += 1;
    }
}

/// 将一段椭圆弧（端点参数化）转为若干三次贝塞尔段
fn arc_to_cubics(
    pb: &mut PathBuilder,
    x1: f32,
    y1: f32,
    mut rx: f32,
    mut ry: f32,
    rot_deg: f32,
    large: bool,
    sweep: bool,
    x2: f32,
    y2: f32,
) {
    if (x1 - x2).abs() < 1e-6 && (y1 - y2).abs() < 1e-6 {
        return;
    }
    rx = rx.abs();
    ry = ry.abs();
    if rx < 1e-6 || ry < 1e-6 {
        pb.line_to(x2, y2);
        return;
    }
    let phi = rot_deg.to_radians();
    let (cp, sp) = (phi.cos(), phi.sin());
    let dx2 = (x1 - x2) / 2.0;
    let dy2 = (y1 - y2) / 2.0;
    let x1p = cp * dx2 + sp * dy2;
    let y1p = -sp * dx2 + cp * dy2;
    let lam = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lam > 1.0 {
        let s = lam.sqrt();
        rx *= s;
        ry *= s;
    }
    let sign = if large == sweep { -1.0 } else { 1.0 };
    let num = (rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p).max(0.0);
    let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let co = sign * (num / den.max(1e-12)).sqrt();
    let cxp = co * (rx * y1p / ry);
    let cyp = co * (-(ry * x1p / rx));
    let cx = cp * cxp - sp * cyp + (x1 + x2) / 2.0;
    let cy = sp * cxp + cp * cyp + (y1 + y2) / 2.0;
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = uy.atan2(ux);
    let cos_t = (ux * vx + uy * vy) / ((ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt());
    let mut delta = cos_t.clamp(-1.0, 1.0).acos();
    if sweep && delta < 0.0 {
        delta += std::f32::consts::TAU;
    } else if !sweep && delta > 0.0 {
        delta -= std::f32::consts::TAU;
    }
    let is_big = delta.abs() >= std::f32::consts::PI - 1e-6;
    if large && !is_big {
        delta = if sweep {
            std::f32::consts::TAU - delta.abs()
        } else {
            -(std::f32::consts::TAU - delta.abs())
        };
    } else if !large && is_big {
        delta = if sweep {
            -(std::f32::consts::TAU - delta.abs())
        } else {
            std::f32::consts::TAU - delta.abs()
        };
    }
    let n = ((delta.abs() / (std::f32::consts::PI / 2.0)).ceil() as usize).max(1);
    let seg = delta / n as f32;
    let t = (4.0 / 3.0) * (seg / 4.0).tan();
    let rot = |x: f32, y: f32| (cp * x - sp * y, sp * x + cp * y);
    for i in 0..n {
        let a0 = theta1 + i as f32 * seg;
        let a1 = a0 + seg;
        let (c0, s0) = (a0.cos(), a0.sin());
        let (c1, s1) = (a1.cos(), a1.sin());
        let p0 = (rx * c0, ry * s0);
        let p1 = (rx * c1, ry * s1);
        let d0 = (-rx * s0 * t, ry * c0 * t);
        let d1 = (-rx * s1 * t, ry * c1 * t);
        let q0 = rot(p0.0, p0.1);
        let q1 = rot(p1.0, p1.1);
        let cd0 = rot(d0.0, d0.1);
        let cd1 = rot(d1.0, d1.1);
        pb.cubic_to(
            cx + q0.0 + cd0.0,
            cy + q0.1 + cd0.1,
            cx + q1.0 + cd1.0,
            cy + q1.1 + cd1.1,
            cx + q1.0,
            cy + q1.1,
        );
    }
}

fn parse(d: &str) -> Option<Path> {
    let b = d.as_bytes();
    let mut pb = PathBuilder::new();
    let mut i = 0usize;
    let (mut cx, mut cy) = (0f32, 0f32);
    let (mut sx, mut sy) = (0f32, 0f32);
    let mut last_cmd: Option<char> = None;
    let mut prev_c2 = (0f32, 0f32); // 上一个 C 的第二个控制点（供 S 反射）

    while i < b.len() {
        skip_ws(b, &mut i);
        if i >= b.len() {
            break;
        }
        let c = b[i] as char;
        let (cmd, rel) = if c.is_ascii_alphabetic() {
            i += 1;
            (Some(c), c.is_ascii_lowercase())
        } else {
            match last_cmd {
                None => return None,
                Some(lc) => (Some(lc), lc.is_ascii_lowercase()),
            }
        };
        let cmd_char = cmd?;
        let upper = cmd_char.to_ascii_uppercase();
        let add = |v: f32, base: f32| if rel { base + v } else { v };

        match upper {
            'M' => {
                // 隐式重复按 L 处理
                let eff = if last_cmd == Some('M') || last_cmd == Some('m') { 'L' } else { 'M' };
                let mut first = true;
                loop {
                    skip_ws(b, &mut i);
                    let x = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let y = parse_number(b, &mut i);
                    let (Some(x), Some(y)) = (x, y) else { break };
                    let (nx, ny) = (add(x, cx), add(y, cy));
                    if eff == 'M' && first {
                        pb.move_to(nx, ny);
                        sx = nx;
                        sy = ny;
                        first = false;
                    } else {
                        pb.line_to(nx, ny);
                    }
                    cx = nx;
                    cy = ny;
                }
                last_cmd = Some(cmd_char);
            }
            'L' => {
                loop {
                    skip_ws(b, &mut i);
                    let x = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let y = parse_number(b, &mut i);
                    let (Some(x), Some(y)) = (x, y) else { break };
                    let (nx, ny) = (add(x, cx), add(y, cy));
                    pb.line_to(nx, ny);
                    cx = nx;
                    cy = ny;
                }
                last_cmd = Some(cmd_char);
            }
            'H' => {
                loop {
                    skip_ws(b, &mut i);
                    let Some(x) = parse_number(b, &mut i) else { break };
                    cx = add(x, cx);
                    pb.line_to(cx, cy);
                }
                last_cmd = Some(cmd_char);
            }
            'V' => {
                loop {
                    skip_ws(b, &mut i);
                    let Some(y) = parse_number(b, &mut i) else { break };
                    cy = add(y, cy);
                    pb.line_to(cx, cy);
                }
                last_cmd = Some(cmd_char);
            }
            'C' => {
                loop {
                    skip_ws(b, &mut i);
                    let x1 = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let y1 = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let x2 = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let y2 = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let x = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let y = parse_number(b, &mut i);
                    let (Some(x1), Some(y1), Some(x2), Some(y2), Some(x), Some(y)) =
                        (x1, y1, x2, y2, x, y)
                    else {
                        break;
                    };
                    let (c1x, c1y) = (add(x1, cx), add(y1, cy));
                    let (c2x, c2y) = (add(x2, cx), add(y2, cy));
                    let (ex, ey) = (add(x, cx), add(y, cy));
                    pb.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
                    prev_c2 = (c2x, c2y);
                    cx = ex;
                    cy = ey;
                }
                last_cmd = Some(cmd_char);
            }
            'S' => {
                loop {
                    skip_ws(b, &mut i);
                    let x2 = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let y2 = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let x = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let y = parse_number(b, &mut i);
                    let (Some(x2), Some(y2), Some(x), Some(y)) = (x2, y2, x, y) else { break };
                    let (c1x, c1y) = (2.0 * cx - prev_c2.0, 2.0 * cy - prev_c2.1);
                    let (c2x, c2y) = (add(x2, cx), add(y2, cy));
                    let (ex, ey) = (add(x, cx), add(y, cy));
                    pb.cubic_to(c1x, c1y, c2x, c2y, ex, ey);
                    prev_c2 = (c2x, c2y);
                    cx = ex;
                    cy = ey;
                }
                last_cmd = Some(cmd_char);
            }
            'A' => {
                loop {
                    skip_ws(b, &mut i);
                    let rx = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let ry = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let rot = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let large = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let sweep = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let x = parse_number(b, &mut i);
                    skip_ws(b, &mut i);
                    let y = parse_number(b, &mut i);
                    let (Some(rx), Some(ry), Some(rot), Some(large), Some(sweep), Some(x), Some(y)) =
                        (rx, ry, rot, large, sweep, x, y)
                    else {
                        break;
                    };
                    let (ex, ey) = (add(x, cx), add(y, cy));
                    arc_to_cubics(&mut pb, cx, cy, rx, ry, rot, large > 0.0, sweep > 0.0, ex, ey);
                    cx = ex;
                    cy = ey;
                }
                last_cmd = Some(cmd_char);
            }
            'Z' => {
                pb.close();
                cx = sx;
                cy = sy;
                last_cmd = Some(cmd_char);
            }
            _ => return None,
        }
    }
    pb.finish()
}

/// 脚步图标路径（懒加载）
pub fn footstep_path() -> &'static Path {
    static P: OnceLock<Option<Path>> = OnceLock::new();
    P.get_or_init(|| parse(FOOTSTEP_D)).as_ref().expect("脚步 SVG 解析失败")
}

/// 子弹图标路径（懒加载）
pub fn bullet_path() -> &'static Path {
    static P: OnceLock<Option<Path>> = OnceLock::new();
    P.get_or_init(|| parse(BULLET_D)).as_ref().expect("子弹 SVG 解析失败")
}

/// 图标变换：viewBox 1024² → 以 (cx,cy) 为中心、size 大小的方形，并按 rotate_deg 旋转
pub fn icon_transform(cx: f32, cy: f32, size: f32, rotate_deg: f32) -> Transform {
    Transform::from_translate(cx, cy)
        .pre_concat(Transform::from_rotate(rotate_deg))
        .pre_concat(Transform::from_scale(size / 1024.0, size / 1024.0))
        .pre_concat(Transform::from_translate(-512.0, -512.0))
}

/// 折线化（供 egui 预览描边）：每个三次曲线细分为 steps 段
pub fn flattened_polylines(path: &Path, steps_per_cubic: usize) -> Vec<Vec<(f32, f32)>> {
    let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut cur: Vec<(f32, f32)> = Vec::new();
    for seg in path.segments() {
        use tiny_skia::PathSegment;
        match seg {
            PathSegment::MoveTo(p) => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push((p.x, p.y));
            }
            PathSegment::LineTo(p) => cur.push((p.x, p.y)),
            PathSegment::QuadTo(c, p) => {
                // 二次曲线转三次再细分
                if let Some(&(lx, ly)) = cur.last() {
                    let c1 = (lx + 2.0 / 3.0 * (c.x - lx), ly + 2.0 / 3.0 * (c.y - ly));
                    let c2 = (p.x + 2.0 / 3.0 * (c.x - p.x), p.y + 2.0 / 3.0 * (c.y - p.y));
                    flatten_cubic(&mut cur, (lx, ly), c1, c2, (p.x, p.y), steps_per_cubic);
                } else {
                    cur.push((p.x, p.y));
                }
            }
            PathSegment::CubicTo(c1, c2, p) => {
                if let Some(&(lx, ly)) = cur.last() {
                    flatten_cubic(
                        &mut cur,
                        (lx, ly),
                        (c1.x, c1.y),
                        (c2.x, c2.y),
                        (p.x, p.y),
                        steps_per_cubic,
                    );
                } else {
                    cur.push((p.x, p.y));
                }
            }
            PathSegment::Close => {
                if let Some(&first) = cur.first() {
                    cur.push(first);
                }
                out.push(std::mem::take(&mut cur));
            }
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn flatten_cubic(
    out: &mut Vec<(f32, f32)>,
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p1: (f32, f32),
    steps: usize,
) {
    let n = steps.max(1);
    for i in 1..=n {
        let t = i as f32 / n as f32;
        let mt = 1.0 - t;
        let x = mt * mt * mt * p0.0
            + 3.0 * mt * mt * t * c1.0
            + 3.0 * mt * t * t * c2.0
            + t * t * t * p1.0;
        let y = mt * mt * mt * p0.1
            + 3.0 * mt * mt * t * c1.1
            + 3.0 * mt * t * t * c2.1
            + t * t * t * p1.1;
        out.push((x, y));
    }
}
