//! Path handling: Lottie bezier paths, shape primitives, flattening, trim and
//! arc-length resampling.
//!
//! Curves are flattened adaptively (like lottie-web's `bez.getSegmentsLength`)
//! and later resampled to a fixed number of points so the exported mesh keeps
//! the same topology for every animation frame.

use serde_json::Value;

use crate::prop;

#[derive(Clone, Debug, Default)]
pub struct PathData {
    pub v: Vec<[f32; 2]>,
    pub i: Vec<[f32; 2]>,
    pub o: Vec<[f32; 2]>,
    pub closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Contour {
    pub pts: Vec<[f32; 2]>,
    pub closed: bool,
}

impl Contour {
    pub fn length(&self) -> f32 {
        if self.pts.len() < 2 {
            return 0.0;
        }
        let mut total = 0.0;
        for i in 0..self.pts.len() - 1 {
            total += dist(self.pts[i], self.pts[i + 1]);
        }
        if self.closed {
            total += dist(*self.pts.last().unwrap(), self.pts[0]);
        }
        total
    }

    pub fn signed_area(&self) -> f32 {
        if self.pts.len() < 3 {
            return 0.0;
        }
        let mut area = 0.0;
        for i in 0..self.pts.len() {
            let a = self.pts[i];
            let b = self.pts[(i + 1) % self.pts.len()];
            area += a[0] * b[1] - b[0] * a[1];
        }
        area * 0.5
    }
}

pub fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    (dx * dx + dy * dy).sqrt()
}

/* ------------------------------------------------------------------ */
/* Lottie path property                                                */
/* ------------------------------------------------------------------ */

/// Evaluate the `ks` property of an `sh` shape at a frame.
pub fn eval_path_property(ks: &Value, frame: f32) -> Option<PathData> {
    let animated = ks.get("a").and_then(Value::as_i64).unwrap_or(0) != 0;
    let k = ks.get("k").unwrap_or(ks);
    if !animated {
        return parse_path_value(k);
    }
    let list = k.as_array()?;
    if list.is_empty() {
        return None;
    }
    if list.len() == 1 {
        return key_path(&list[0]);
    }

    let first_t = key_time(&list[0]);
    if frame <= first_t {
        return key_path(&list[0]);
    }

    let mut index = 0;
    for i in 0..list.len() - 1 {
        if frame >= key_time(&list[i]) {
            index = i;
        } else {
            break;
        }
    }
    if index >= list.len() - 1 {
        return key_path(&list[list.len() - 1]);
    }

    let start = &list[index];
    let next = &list[index + 1];
    let t0 = key_time(start);
    let t1 = key_time(next);
    let progress = if t1 > t0 {
        ((frame - t0) / (t1 - t0)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let held = start.get("h").and_then(Value::as_i64).unwrap_or(0) == 1;
    let a = key_path(start);
    // lottie-web: `nextKeyData.s ? nextKeyData.s[0] : keyData.e[0]`.
    let b = key_path(next).or_else(|| end_path(start));
    if held {
        return a;
    }
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) => (a, b),
        (a, b) => return a.or(b),
    };
    if a.v.len() != b.v.len() {
        return Some(if progress < 0.5 { a } else { b });
    }

    let ease = prop::cubic_ease(
        start
            .get("o")
            .and_then(|o| o.get("x"))
            .and_then(Value::as_f64)
            .unwrap_or(0.333) as f32,
        start
            .get("o")
            .and_then(|o| o.get("y"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0) as f32,
        start
            .get("i")
            .and_then(|i| i.get("x"))
            .and_then(Value::as_f64)
            .unwrap_or(0.667) as f32,
        start
            .get("i")
            .and_then(|i| i.get("y"))
            .and_then(Value::as_f64)
            .unwrap_or(1.0) as f32,
        progress,
    );

    let mut out = a.clone();
    for idx in 0..out.v.len() {
        for axis in 0..2 {
            out.v[idx][axis] = lerp(a.v[idx][axis], b.v[idx][axis], ease);
            out.i[idx][axis] = lerp(a.i[idx][axis], b.i[idx][axis], ease);
            out.o[idx][axis] = lerp(a.o[idx][axis], b.o[idx][axis], ease);
        }
    }
    Some(out)
}

fn key_time(key: &Value) -> f32 {
    key.get("t").and_then(Value::as_f64).unwrap_or(0.0) as f32
}

/// Animated shape keyframes wrap the path in a one-element array (`s[0]`),
/// while static properties store the path object directly.
fn parse_path_value(value: &Value) -> Option<PathData> {
    match value {
        Value::Array(items) => items.first().and_then(parse_path),
        other => parse_path(other),
    }
}

fn key_path(key: &Value) -> Option<PathData> {
    key.get("s").and_then(parse_path_value)
}

fn end_path(key: &Value) -> Option<PathData> {
    key.get("e").and_then(parse_path_value)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn parse_path(value: &Value) -> Option<PathData> {
    let v = parse_points(value.get("v")?)?;
    let i = value
        .get("i")
        .and_then(parse_points)
        .unwrap_or_else(|| vec![[0.0; 2]; v.len()]);
    let o = value
        .get("o")
        .and_then(parse_points)
        .unwrap_or_else(|| vec![[0.0; 2]; v.len()]);
    if v.is_empty() || i.len() < v.len() || o.len() < v.len() {
        return None;
    }
    Some(PathData {
        v,
        i,
        o,
        closed: value.get("c").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn parse_points(value: &Value) -> Option<Vec<[f32; 2]>> {
    let items = value.as_array()?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let arr = item.as_array()?;
        if arr.len() < 2 {
            return None;
        }
        out.push([arr[0].as_f64()? as f32, arr[1].as_f64()? as f32]);
    }
    Some(out)
}

/* ------------------------------------------------------------------ */
/* Flattening                                                          */
/* ------------------------------------------------------------------ */

pub fn flatten_path(path: &PathData) -> Contour {
    let n = path.v.len();
    if n < 2 {
        return Contour {
            pts: path.v.clone(),
            closed: path.closed,
        };
    }
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(n * 8);
    pts.push(path.v[0]);
    let segments = if path.closed { n } else { n - 1 };
    for s in 0..segments {
        let p0 = path.v[s];
        let p1 = path.v[(s + 1) % n];
        let c0 = [
            p0[0] + path.o[s][0],
            p0[1] + path.o[s][1],
        ];
        let c1 = [
            p1[0] + path.i[(s + 1) % n][0],
            p1[1] + path.i[(s + 1) % n][1],
        ];
        flatten_cubic(p0, c0, c1, p1, 0, &mut pts);
    }
    // Remove duplicated closing point if the last sample equals the first.
    if path.closed && pts.len() > 1 {
        let first = pts[0];
        let last = *pts.last().unwrap();
        if dist(first, last) < 1e-4 {
            pts.pop();
        }
    }
    Contour {
        pts,
        closed: path.closed,
    }
}

fn flatten_cubic(
    p0: [f32; 2],
    p1: [f32; 2],
    p2: [f32; 2],
    p3: [f32; 2],
    depth: u32,
    out: &mut Vec<[f32; 2]>,
) {
    if depth >= 8 || cubic_flat(p0, p1, p2, p3, 0.2) {
        out.push(p3);
        return;
    }
    let p01 = midpoint(p0, p1);
    let p12 = midpoint(p1, p2);
    let p23 = midpoint(p2, p3);
    let p012 = midpoint(p01, p12);
    let p123 = midpoint(p12, p23);
    let mid = midpoint(p012, p123);
    flatten_cubic(p0, p01, p012, mid, depth + 1, out);
    flatten_cubic(mid, p123, p23, p3, depth + 1, out);
}

fn midpoint(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

fn cubic_flat(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], tolerance: f32) -> bool {
    let dx = p3[0] - p0[0];
    let dy = p3[1] - p0[1];
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-4 {
        let d1 = dist(p0, p1).max(dist(p0, p2));
        return d1 < tolerance;
    }
    let d1 = ((p1[0] - p0[0]) * dy - (p1[1] - p0[1]) * dx).abs() / len;
    let d2 = ((p2[0] - p0[0]) * dy - (p2[1] - p0[1]) * dx).abs() / len;
    d1.max(d2) < tolerance
}

/* ------------------------------------------------------------------ */
/* Shape primitives                                                    */
/* ------------------------------------------------------------------ */

pub fn ellipse_contour(center: [f32; 2], size: [f32; 2], direction: i32) -> Contour {
    let rx = (size[0] * 0.5).abs();
    let ry = (size[1] * 0.5).abs();
    if rx < 1e-4 || ry < 1e-4 {
        return Contour {
            pts: vec![center],
            closed: true,
        };
    }
    const KAPPA: f32 = 0.552_915;
    let mut pts = Vec::with_capacity(4);
    pts.push([center[0] + rx, center[1]]);
    pts.push([center[0], center[1] + ry]);
    pts.push([center[0] - rx, center[1]]);
    pts.push([center[0], center[1] - ry]);
    let path = PathData {
        v: pts,
        i: vec![
            [0.0, -ry * KAPPA],
            [rx * KAPPA, 0.0],
            [0.0, ry * KAPPA],
            [-rx * KAPPA, 0.0],
        ],
        o: vec![
            [0.0, ry * KAPPA],
            [-rx * KAPPA, 0.0],
            [0.0, -ry * KAPPA],
            [rx * KAPPA, 0.0],
        ],
        closed: true,
    };
    let mut contour = flatten_path(&path);
    if direction < 0 {
        contour.pts.reverse();
    }
    contour
}

pub fn rect_contour(center: [f32; 2], size: [f32; 2], radius: [f32; 2], direction: i32) -> Contour {
    let hw = (size[0] * 0.5).abs();
    let hh = (size[1] * 0.5).abs();
    if hw < 1e-4 || hh < 1e-4 {
        return Contour {
            pts: vec![center],
            closed: true,
        };
    }
    let rx = radius[0].abs().min(hw);
    let ry = radius[1].abs().min(hh);
    if rx <= 0.01 && ry <= 0.01 {
        let mut pts = vec![
            [center[0] + hw, center[1] - hh],
            [center[0] + hw, center[1] + hh],
            [center[0] - hw, center[1] + hh],
            [center[0] - hw, center[1] - hh],
        ];
        if direction < 0 {
            pts.reverse();
        }
        return Contour { pts, closed: true };
    }

    // Clockwise in screen coordinates (y grows downwards).
    const ARC_STEPS: usize = 6;
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(8 + ARC_STEPS * 4);
    let corners = [
        // (corner point, arc center, start angle)
        (
            [center[0] + hw - rx, center[1] - hh],
            [center[0] + hw - rx, center[1] - hh + ry],
            -std::f32::consts::FRAC_PI_2,
        ),
        (
            [center[0] + hw, center[1] + hh - ry],
            [center[0] + hw - rx, center[1] + hh - ry],
            0.0,
        ),
        (
            [center[0] - hw + rx, center[1] + hh],
            [center[0] - hw + rx, center[1] + hh - ry],
            std::f32::consts::FRAC_PI_2,
        ),
        (
            [center[0] - hw, center[1] - hh + ry],
            [center[0] - hw + rx, center[1] - hh + ry],
            std::f32::consts::PI,
        ),
    ];
    for (point, arc_center, start) in corners.iter() {
        pts.push(*point);
        for s in 1..ARC_STEPS {
            let t = *start + std::f32::consts::FRAC_PI_2 * (s as f32 / ARC_STEPS as f32);
            pts.push([arc_center[0] + t.cos() * rx, arc_center[1] + t.sin() * ry]);
        }
    }
    if direction < 0 {
        pts.reverse();
    }
    Contour { pts, closed: true }
}

pub fn polystar_contour(
    center: [f32; 2],
    points: f32,
    outer: f32,
    inner: f32,
    rotation_deg: f32,
    star: bool,
    direction: i32,
) -> Contour {
    let n = points.round().max(2.0) as usize;
    let angle = rotation_deg.to_radians() - std::f32::consts::FRAC_PI_2;
    let mut pts = Vec::with_capacity(n * 2);
    if star {
        for i in 0..n * 2 {
            let radius = if i % 2 == 0 { outer } else { inner };
            let a = angle + std::f32::consts::PI * (i as f32) / n as f32;
            pts.push([center[0] + a.cos() * radius, center[1] + a.sin() * radius]);
        }
    } else {
        for i in 0..n {
            let a = angle + std::f32::consts::TAU * (i as f32) / n as f32;
            pts.push([center[0] + a.cos() * outer, center[1] + a.sin() * outer]);
        }
    }
    if direction < 0 {
        pts.reverse();
    }
    Contour { pts, closed: true }
}

/* ------------------------------------------------------------------ */
/* Resampling                                                          */
/* ------------------------------------------------------------------ */

/// Resample a contour to exactly `k` points by arc length.
pub fn resample(contour: &Contour, k: usize, closed: bool) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    resample_into(contour, k, closed, &mut out);
    out
}

/// Same as [`resample`], but writes into a reusable buffer. The arc-length
/// walk is streamed, so no temporary segment table is allocated.
pub fn resample_into(contour: &Contour, k: usize, closed: bool, out: &mut Vec<[f32; 2]>) {
    out.clear();
    let k = k.max(2);
    let pts = &contour.pts;
    if pts.len() < 2 {
        out.resize(k, pts.first().copied().unwrap_or([0.0, 0.0]));
        return;
    }
    let total = contour.length();
    if total <= 1e-6 {
        out.resize(k, pts[0]);
        return;
    }
    let segments = if closed { pts.len() } else { pts.len() - 1 };
    out.reserve(k);

    let mut cursor = 0usize;
    let mut seg_start = 0.0f32;
    let mut seg_len = dist(pts[0], pts[1 % pts.len()]);
    for j in 0..k {
        let target = if closed {
            total * (j as f32 / k as f32)
        } else {
            total * (j as f32 / (k - 1) as f32)
        };
        while cursor + 1 < segments && seg_start + seg_len < target {
            seg_start += seg_len;
            cursor += 1;
            seg_len = dist(pts[cursor], pts[(cursor + 1) % pts.len()]);
        }
        let a = pts[cursor];
        let b = pts[(cursor + 1) % pts.len()];
        let t = if seg_len > 1e-6 {
            ((target - seg_start) / seg_len).clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
}

/// Extract `length_frac` of the contour starting at `start_frac`.
/// Returns an open contour (the caller may close it for fills).
pub fn trim_segment(contour: &Contour, start_frac: f32, length_frac: f32) -> Contour {
    let start = start_frac.rem_euclid(1.0);
    let length = length_frac.clamp(0.0, 1.0);
    if length >= 0.999 {
        return contour.clone();
    }
    let pts = &contour.pts;
    if length <= 1e-5 || pts.len() < 2 {
        return Contour {
            pts: vec![pts.first().copied().unwrap_or([0.0, 0.0])],
            closed: false,
        };
    }
    let total = contour.length();
    if total <= 1e-6 {
        return Contour {
            pts: vec![pts[0]],
            closed: false,
        };
    }
    let segments = if contour.closed {
        pts.len()
    } else {
        pts.len() - 1
    };
    // Cumulative arc lengths computed once instead of per sample point.
    let mut cumulative = Vec::with_capacity(segments);
    let mut accumulated = 0.0f32;
    for s in 0..segments {
        accumulated += dist(pts[s], pts[(s + 1) % pts.len()]);
        cumulative.push(accumulated);
    }

    let start_len = total * start;
    let end_len = start_len + total * length;
    let steps = 48usize;
    let mut out = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let d = start_len + (end_len - start_len) * (i as f32 / steps as f32);
        let target = d.rem_euclid(total);
        let cursor = cumulative
            .partition_point(|value| *value < target)
            .min(segments - 1);
        let prev = if cursor == 0 { 0.0 } else { cumulative[cursor - 1] };
        let seg_len = cumulative[cursor] - prev;
        let t = if seg_len > 1e-6 {
            ((target - prev) / seg_len).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let a = pts[cursor];
        let b = pts[(cursor + 1) % pts.len()];
        out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
    Contour {
        pts: out,
        closed: false,
    }
}

/// Split a trim range into up to two fixed slots so topology stays stable
/// when the trimmed range wraps around the contour.
pub fn trim_slots(s: f32, e: f32, offset: f32) -> [(f32, f32); 2] {
    let start = s.clamp(0.0, 1.0);
    let end = e.clamp(0.0, 1.0);
    let (a, b) = if start <= end {
        (start, end)
    } else {
        (end, start)
    };
    let length = b - a;
    if length >= 0.999 {
        return [(0.0, 1.0), (0.0, 0.0)];
    }
    let begin = (a + offset.rem_euclid(1.0)).rem_euclid(1.0);
    let first_len = length.min(1.0 - begin);
    let second_len = (length - first_len).max(0.0);
    [(begin, first_len), (0.0, second_len)]
}
