//! Lottie 属性求值，语义对齐 lottie-web 的 `PropertyFactory`。
//!
//! 模块职责：把 `{ a, k }` 形式的属性对象在指定帧上求值为 f32 分量数组，
//! 供 `shape`（变换、颜色、trim 等）与 `path`（形状路径）复用。
//!
//! 关键语义：
//! * 静态值原样返回；关键帧属性由 `k` 首元素是否为对象判定（见
//!   [`is_keyframed`]），与 lottie-web 一致，不依赖可能缺失的 `a` 字段；
//! * 关键帧值用 cubic-bezier 时间曲线缓动；
//! * 空间关键帧（含 `to` / `ti`）在位置空间走三次贝塞尔，并按**弧长**采样，
//!   与 lottie-web 的 `bez.buildBezierData` 一致；
//! * `h === 1` 的关键帧保持起始值不插值。
//!
//! 单位约定：`frame` 为帧号；返回值各分量保持属性的原始单位（位置为像素、
//! 透明度为 0..100、缩放为百分比），由调用方负责归一化。

use serde_json::Value;

/// 求值一个 Lottie 属性对象 `{ a, k }`。
///
/// `frame` 为帧号；关键帧属性见 [`is_keyframed`]，否则按静态值解析。
/// 返回值分量顺序与 Lottie 定义一致（如位置为 x, y, z）。
pub fn eval(value: &Value, frame: f32) -> Vec<f32> {
    let k = value.get("k").unwrap_or(value);
    if is_keyframed(value) {
        eval_keyframes(k, frame)
    } else {
        eval_static(k)
    }
}

/// 属性是否按关键帧求值。
///
/// 对齐 lottie-web 的 `PropertyFactory.getProp`：`k` 是数组且**首元素不是数字**
/// （即关键帧对象）时才走关键帧分支；`k` 为数组首元素是数字（静态多维值）、
/// 空数组或直接是值对象时都按静态值解析。不能只看 `a` 字段——部分导出文件
/// （如 `dalek.json`）省略了 `a`，但 `k` 仍然直接存放关键帧数组。
pub fn is_keyframed(value: &Value) -> bool {
    match value.get("k") {
        Some(Value::Array(items)) => match items.first() {
            Some(Value::Number(_)) | None => false,
            Some(_) => true,
        },
        _ => false,
    }
}

/// 求值 `parent[key]`；字段缺失或求值为空时返回 `fallback` 的拷贝。
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

/// 求值并只取第一个分量（用于透明度、线宽等标量属性）。
pub fn eval_scalar(parent: &Value, key: &str, frame: f32, fallback: f32) -> f32 {
    eval_in(parent, key, frame, &[fallback])
        .first()
        .copied()
        .unwrap_or(fallback)
}

/// 解析静态值：支持数字、数字数组、梯度 stop 对象及 `{ v }` / `{ s }` 包装。
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

/// 关键帧求值主流程。
///
/// 边界行为与 lottie-web 一致：首帧之前返回首帧值；末帧之后返回末帧值
/// （优先 `s`，为空再用 `e`）；区间内先算线性进度 `progress`，再经
/// cubic-bezier 缓冲曲线得到缓动进度。
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

    // 找到包含当前帧的区间：取最后一个 `t <= frame` 的关键帧。
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

    // 区间起点 keyframe 的 `s` 为起始值；区间终点优先用下一帧的 `s`
    // （Lottie 关键帧链用下一帧 `s` 作为本段终值），缺失时退回本帧 `e`。
    let s = key_start(start);
    let e = {
        let next_start = key_start(next);
        if next_start.is_empty() {
            key_end(start)
        } else {
            next_start
        }
    };
    // lottie-web 在 `h === 1` 时保持当前值；普通关键帧省略该字段。
    let held = start.get("h").and_then(Value::as_i64).unwrap_or(0) == 1;

    if held {
        return if s.is_empty() { e } else { s };
    }

    // 空间关键帧：`to` / `ti` 是相对起终点的控制点偏移，走三维贝塞尔。
    if let (Some(to), Some(ti)) = (start.get("to"), start.get("ti")) {
        let to = as_points(to);
        let ti = as_points(ti);
        if !to.is_empty() && !ti.is_empty() && s.len() >= 2 && e.len() >= 2 {
            let ease = ease_for_dim(start, 0, progress);
            return spatial_value(&s, &e, &to, &ti, ease);
        }
    }

    // 普通（非空间）关键帧：逐分量按各自维度的缓动曲线做标量插值。
    let mut out = vec![0.0f32; s.len().min(e.len())];
    let dims = out.len();
    for i in 0..dims {
        let p = ease_for_dim(start, i, progress);
        out[i] = s[i] + (e[i] - s[i]) * p;
    }
    out
}

/// 读取关键帧的 `t`（帧号），缺失时按 0 处理。
fn time_of(key: &Value) -> f32 {
    key.get("t").and_then(Value::as_f64).unwrap_or(0.0) as f32
}

/// 取关键帧起始值：优先 `s`，为空时退回 `e`。
fn key_start(key: &Value) -> Vec<f32> {
    key.get("s")
        .map(eval_static)
        .filter(|v| !v.is_empty())
        .or_else(|| key.get("e").map(eval_static).filter(|v| !v.is_empty()))
        .unwrap_or_default()
}

/// 取关键帧结束值 `e`，缺失时返回空数组。
fn key_end(key: &Value) -> Vec<f32> {
    key.get("e").map(eval_static).unwrap_or_default()
}

/// 把 `[x, y, ...]` 形式的 JSON 数组解析为 f32 向量。
fn as_points(value: &Value) -> Vec<f32> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(Value::as_f64).map(|n| n as f32).collect())
        .unwrap_or_default()
}

/// 解析某个维度的 cubic-bezier 控制点（出/入切线）。
///
/// `o` / `i` 的 `x` / `y` 既可能是标量也可能是按维度展开的数组；
/// 缺省值取 lottie-web 的 `(0.333, 0, 0.667, 1)`。
fn ease_for_dim(key: &Value, dim: usize, progress: f32) -> f32 {
    let out_x = component(key.get("o"), "x", dim, 0.333);
    let out_y = component(key.get("o"), "y", dim, 0.0);
    let in_x = component(key.get("i"), "x", dim, 0.667);
    let in_y = component(key.get("i"), "y", dim, 1.0);
    cubic_ease(out_x, out_y, in_x, in_y, progress)
}

/// 读取缓动对象的某个轴分量；数组按 `dim` 取，越界时退回第 0 项。
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

/// cubic-bezier 时间曲线：先在 X 轴二分求解 `x(u) = t`，再返回 `y(u)`。
///
/// 与 lottie-web 相同，用 18 次二分逼近，保证单调且无需解三次方程；
/// `t` 超出 `[0, 1]` 时钳制到端点。
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

/// 一维三次贝塞尔分量：固定端点 0 / 1，控制点为 `p1`、`p2`。
fn sample_bezier(p1: f32, p2: f32, t: f32) -> f32 {
    let it = 1.0 - t;
    3.0 * it * it * t * p1 + 3.0 * it * t * t * p2 + t * t * t
}

/// 在空间三次贝塞尔 `s -> e` 上按缓动后的**弧长比例**取点。
///
/// 控制点 `c1 = s + to`、`c2 = e + ti`；先按 150 段折线近似曲线并累加
/// 弦长，再按 `progress` 找到对应弧长处线性插值。lottie-web 默认就是
/// 150 段，这里保持同样分辨率。返回值维度取 `s`/`e` 的较小者，最多 3 维。
fn spatial_value(s: &[f32], e: &[f32], to: &[f32], ti: &[f32], progress: f32) -> Vec<f32> {
    const STEPS: usize = 150;
    let dims = s.len().min(e.len()).min(3);
    let mut points = [[0.0f32; 3]; STEPS + 1];
    // 逐段采样三次贝塞尔， Bernstein 基函数权重 b0..b3。
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

    // 累加相邻采样点的欧氏距离，得到弧长表。
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
    // 目标弧长 = 总长 * progress；定位到所在折线段后做局部线性插值。
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
