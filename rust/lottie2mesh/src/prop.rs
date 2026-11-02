//! Lottie property evaluation.
//!
//! Mirrors the semantics of lottie-web's `PropertyFactory`:
//! * static values (`a = 0`) are returned as-is;
//! * keyframed values (`a = 1`) are eased with a cubic-bezier timing function;
//! * spatial keyframes (`to` / `ti`) follow a cubic path in position space and
//!   are sampled by arc length, exactly like `bez.buildBezierData`.

use serde_json::Value;

/// Evaluate a Lottie property object `{ a, k }` at the given frame.
pub fn eval(value: &Value, frame: f32) -> Vec<f32> {
    let animated = value
        .get("a")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        != 0;
    let k = value.get("k").unwrap_or(value);
    if animated {
        eval_keyframes(k, frame)
    } else {
        eval_static(k)
    }
}

/// Evaluate a property found under `key` inside `parent`, falling back to `fallback`.
pub fn eval_in(parent: &Value, key: &str, frame: f32, fallback: &[f32]) -> Vec<f32> {
    match parent.get(key) {
        Some(value) => {
            let out = eval(value, frame);
            if out.is_empty() {
                fallback.to_vec()
            } else {
                out
            }
        }
        None => fallback.to_vec(),
    }
}

/// Evaluate a property and return only its first component.
pub fn eval_scalar(parent: &Value, key: &str, frame: f32, fallback: f32) -> f32 {
    eval_in(parent, key, frame, &[fallback])
        .first()
        .copied()
        .unwrap_or(fallback)
}

fn eval_static(k: &Value) -> Vec<f32> {
    match k {
        Value::Number(n) => vec![n.as_f64().unwrap_or(0.0) as f32],
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                if let Some(n) = item.as_f64() {
                    out.push(n as f32);
                } else if let Some(n) = item.get("s").and_then(Value::as_f64) {
                    // Gradient stop value.
                    out.push(n as f32);
                }
            }
            out
        }
        Value::Object(map) => {
            if let Some(v) = map.get("v").and_then(Value::as_array) {
                v.iter().filter_map(Value::as_f64).map(|n| n as f32).collect()
            } else if let Some(s) = map.get("s").and_then(Value::as_array) {
                s.iter().filter_map(Value::as_f64).map(|n| n as f32).collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn eval_keyframes(frames: &Value, frame: f32) -> Vec<f32> {
    let Some(list) = frames.as_array() else {
        return eval_static(frames);
    };
    if list.is_empty() {
        return Vec::new();
    }
    if list.len() == 1 {
        return key_start(&list[0]);
    }

    // Before the first keyframe.
    let first_t = time_of(&list[0]);
    if frame <= first_t {
        return key_start(&list[0]);
    }

    // Find the segment containing the frame.
    let mut index = 0usize;
    for i in 0..list.len() - 1 {
        if frame >= time_of(&list[i]) {
            index = i;
        } else {
            break;
        }
    }

    if index >= list.len() - 1 {
        let last = &list[list.len() - 1];
        let value = key_start(last);
        return if value.is_empty() { key_end(last) } else { value };
    }

    let start = &list[index];
    let next = &list[index + 1];
    let t0 = time_of(start);
    let t1 = time_of(next);
    let progress = if t1 > t0 {
        ((frame - t0) / (t1 - t0)).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let s = key_start(start);
    let e = {
        let next_start = key_start(next);
        if next_start.is_empty() {
            key_end(start)
        } else {
            next_start
        }
    };
    // lottie-web holds when `h === 1`; ordinary keyframes omit it.
    let held = start.get("h").and_then(Value::as_i64).unwrap_or(0) == 1;

    if held {
        return if s.is_empty() { e } else { s };
    }

    if let (Some(to), Some(ti)) = (start.get("to"), start.get("ti")) {
        let to = as_points(to);
        let ti = as_points(ti);
        if !to.is_empty() && !ti.is_empty() && s.len() >= 2 && e.len() >= 2 {
            let ease = ease_for_dim(start, 0, progress);
            return spatial_value(&s, &e, &to, &ti, ease);
        }
    }

    let mut out = vec![0.0f32; s.len().min(e.len())];
    let dims = out.len();
    for i in 0..dims {
        let p = ease_for_dim(start, i, progress);
        out[i] = s[i] + (e[i] - s[i]) * p;
    }
    out
}

fn time_of(key: &Value) -> f32 {
    key.get("t").and_then(Value::as_f64).unwrap_or(0.0) as f32
}

fn key_start(key: &Value) -> Vec<f32> {
    key.get("s")
        .map(eval_static)
        .filter(|v| !v.is_empty())
        .or_else(|| key.get("e").map(eval_static).filter(|v| !v.is_empty()))
        .unwrap_or_default()
}

fn key_end(key: &Value) -> Vec<f32> {
    key.get("e").map(eval_static).unwrap_or_default()
}

fn as_points(value: &Value) -> Vec<f32> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_f64).map(|n| n as f32).collect())
        .unwrap_or_default()
}

/// Resolve the cubic-bezier control points for a given dimension.
fn ease_for_dim(key: &Value, dim: usize, progress: f32) -> f32 {
    let out_x = component(key.get("o"), "x", dim, 0.333);
    let out_y = component(key.get("o"), "y", dim, 0.0);
    let in_x = component(key.get("i"), "x", dim, 0.667);
    let in_y = component(key.get("i"), "y", dim, 1.0);
    cubic_ease(out_x, out_y, in_x, in_y, progress)
}

fn component(ease: Option<&Value>, axis: &str, dim: usize, fallback: f32) -> f32 {
    let Some(ease) = ease else { return fallback };
    let Some(value) = ease.get(axis) else { return fallback };
    if let Some(n) = value.as_f64() {
        return n as f32;
    }
    if let Some(items) = value.as_array() {
        return items
            .get(dim)
            .or_else(|| items.first())
            .and_then(Value::as_f64)
            .map(|n| n as f32)
            .unwrap_or(fallback);
    }
    fallback
}

/// Cubic-bezier timing function, solved on the X axis (same as lottie-web).
pub fn cubic_ease(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    let mut mid = t;
    for _ in 0..18 {
        mid = (lo + hi) * 0.5;
        let x = sample_bezier(x1, x2, mid);
        if x < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    sample_bezier(y1, y2, mid)
}

fn sample_bezier(p1: f32, p2: f32, t: f32) -> f32 {
    let it = 1.0 - t;
    3.0 * it * it * t * p1 + 3.0 * it * t * t * p2 + t * t * t
}

/// Sample the spatial cubic through `s -> e` at the eased arc length.
/// lottie-web uses 150 curve segments by default, so match that resolution.
fn spatial_value(s: &[f32], e: &[f32], to: &[f32], ti: &[f32], progress: f32) -> Vec<f32> {
    const STEPS: usize = 150;
    let dims = s.len().min(e.len()).min(3);
    let mut points = [[0.0f32; 3]; STEPS + 1];
    for (step, point) in points.iter_mut().enumerate() {
        let t = step as f32 / STEPS as f32;
        let it = 1.0 - t;
        let b0 = it * it * it;
        let b1 = 3.0 * it * it * t;
        let b2 = 3.0 * it * t * t;
        let b3 = t * t * t;
        for (d, value) in point.iter_mut().enumerate().take(dims) {
            let c1 = s[d] + to.get(d).copied().unwrap_or(0.0);
            let c2 = e[d] + ti.get(d).copied().unwrap_or(0.0);
            *value = b0 * s[d] + b1 * c1 + b2 * c2 + b3 * e[d];
        }
    }

    // Build cumulative arc lengths.
    let mut lengths = [0.0f32; STEPS + 1];
    for i in 1..=STEPS {
        let mut d = 0.0f32;
        for axis in 0..dims {
            let delta = points[i][axis] - points[i - 1][axis];
            d += delta * delta;
        }
        lengths[i] = lengths[i - 1] + d.sqrt();
    }
    let total = lengths[STEPS];
    if total <= 1e-6 {
        return points[0][..dims].to_vec();
    }
    let target = total * progress;
    let mut i = 1usize;
    while i < lengths.len() && lengths[i] < target {
        i += 1;
    }
    let i = i.min(points.len() - 1);
    let prev_len = lengths[i - 1];
    let seg_len = (lengths[i] - prev_len).max(1e-6);
    let local = (target - prev_len) / seg_len;
    let mut out = vec![0.0f32; dims];
    for axis in 0..dims {
        out[axis] = points[i - 1][axis] + (points[i][axis] - points[i - 1][axis]) * local;
    }
    out
}
