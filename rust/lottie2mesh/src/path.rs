//! 路径处理：Lottie 贝塞尔路径、形状图元、扁平化、trim 与弧长重采样。
//!
//! 模块职责：
//! * 把 Lottie 的 `v` / `i` / `o` 贝塞尔路径扁平化为折线 `Contour`；
//! * 生成矩形、椭圆、多角星等图元轮廓；
//! * 按弧长把轮廓重采样到固定点数 `k`，使导出网格在每一帧保持相同拓扑；
//! * 实现 trim path 的区间截取，并把区间拆成固定槽位以稳定拓扑。
//!
//! 单位约定：所有坐标、长度均为 Lottie 像素；`start_frac` / `length_frac`
//! 是 0..1 的归一化弧长比例，`k` 为每轮廓的重采样点数。
//!
//! 曲线采用自适应细分（参考 lottie-web 的 `bez.getSegmentsLength`）：先用
//! 递归 de Casteljau 逼近，直到控制点到弦距离低于容差或达到递归上限。

use serde_json::Value;

use crate::prop;

/// Lottie 原始贝塞尔路径：顶点 + 进入/离开控制点偏移。
///
/// * `v[i]`：第 i 个顶点，单位像素；
/// * `i[i]`：顶点 i 的**进入**控制点相对 `v[i]` 的偏移；
/// * `o[i]`：顶点 i 的**离开**控制点相对 `v[i]` 的偏移；
/// * `closed`：是否闭合（闭合时最后一段连回第 0 点）。
#[derive(Clone, Debug, Default)]
pub struct PathData {
    pub v: Vec<[f32; 2]>,
    pub i: Vec<[f32; 2]>,
    pub o: Vec<[f32; 2]>,
    pub closed: bool,
}

/// 扁平化后的二维折线轮廓，坐标为 Lottie 像素。
#[derive(Clone, Debug, Default)]
pub struct Contour {
    pub pts: Vec<[f32; 2]>,
    pub closed: bool,
}

impl Contour {
    /// 轮廓总长度（像素）；闭合轮廓额外累加末点到首点的一段。
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

    /// 有符号面积（鞋带公式）；正负号表示顶点绕向，绝对值用于孔洞包含判断。
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

/// 两点欧氏距离（像素）。
pub fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    (dx * dx + dy * dy).sqrt()
}

/* ------------------------------------------------------------------ */
/* Lottie path property                                                */
/* ------------------------------------------------------------------ */

/// 在指定帧求值 `sh` 形状的 `ks` 路径属性。
///
/// `frame` 为图层局部帧号。支持静态路径与关键帧路径：
/// * 关键帧按与 `prop` 相同的区间查找与 cubic-bezier 缓动；
/// * 路径 morph 逐顶点、逐分量线性插值 `v` / `i` / `o`（控制点偏移同理）；
/// * 两帧顶点数不一致时无法 morph，按进度取较近的一帧。
/// 返回 `None` 表示路径为空或格式不合法。
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

    // 与 prop 一致：定位最后一个 `t <= frame` 的关键帧区间。
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
    // lottie-web 的终值取法：`nextKeyData.s ? nextKeyData.s[0] : keyData.e[0]`。
    let b = key_path(next).or_else(|| end_path(start));
    if held {
        return a;
    }
    let (a, b) = match (a, b) {
        (Some(a), Some(b)) => (a, b),
        (a, b) => return a.or(b),
    };
    if a.v.len() != b.v.len() {
        // 顶点数不同（拓扑变化）时无法逐点 morph，按进度硬切到较近一帧。
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

    // 路径 morph：对顶点坐标与进出控制点偏移逐分量插值。
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

/// 读取路径关键帧的 `t`（帧号），缺失时按 0。
fn key_time(key: &Value) -> f32 {
    key.get("t").and_then(Value::as_f64).unwrap_or(0.0) as f32
}

/// 动画路径关键帧把路径包在单元素数组里（`s[0]`），静态属性则直接是路径对象。
fn parse_path_value(value: &Value) -> Option<PathData> {
    match value {
        Value::Array(items) => items.first().and_then(parse_path),
        other => parse_path(other),
    }
}

/// 读取关键帧起始路径 `s`。
fn key_path(key: &Value) -> Option<PathData> {
    key.get("s").and_then(parse_path_value)
}

/// 读取关键帧结束路径 `e`（仅当下一帧没有 `s` 时作为 morph 终值）。
fn end_path(key: &Value) -> Option<PathData> {
    key.get("e").and_then(parse_path_value)
}

/// 标量线性插值：`t = 0` 返回 `a`，`t = 1` 返回 `b`。
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// 解析一个路径对象；`i` / `o` 缺失时用零偏移补齐。
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

/// 解析 `[[x, y], ...]` 顶点数组；任一元素不足两维则判定失败。
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

/// 把 Lottie 贝塞尔路径扁平化为折线轮廓。
///
/// 每段以 `v[s]` 为起点、`v[s+1]` 为终点，控制点分别为 `v[s] + o[s]` 与
/// `v[s+1] + i[s+1]`；闭合路径多处理最后一段 `v[n-1] -> v[0]`。
/// 输出的 `pts` 不重复首点，闭合关系由 `closed` 标志表达。
pub fn flatten_path(path: &PathData) -> Contour {
    let n = path.v.len();
    if n < 2 {
        return Contour {
            pts: path.v.clone(),
            closed: path.closed,
        };
    }
    // 经验容量：每段平均细分出约 8 个采样点。
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
    // 去掉与首点重合的末尾采样点（闭合段回写到起点时会产生重复点）。
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

/// 递归细分一段三次贝塞尔（de Casteljau 二分）。
///
/// 当递归深度达到 8 或两控制点到弦的距离都小于 0.2 像素时停止，
/// 把终点 `p3` 追加到 `out`（起点由调用方预先写入），从而避免重复点。
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

/// 两点中点。
fn midpoint(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
}

/// 判断三次贝塞尔是否已足够平直（控制点到首尾弦的垂距均小于 `tolerance`）。
///
/// 弦退化（首尾几乎重合）时改用控制点间的最大距离判断，单位像素。
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

/// 用四段三次贝塞尔近似椭圆，再扁平化为轮廓。
///
/// `center` / `size` 单位为像素，半径取 `size / 2` 的绝对值；`KAPPA` 是
/// 圆弧到贝塞尔的标准控制点系数。`direction < 0` 时反转顶点顺序。
pub fn ellipse_contour(center: [f32; 2], size: [f32; 2], direction: i32) -> Contour {
    let rx = (size[0] * 0.5).abs();
    let ry = (size[1] * 0.5).abs();
    if rx < 1e-4 || ry < 1e-4 {
        return Contour {
            pts: vec![center],
            closed: true,
        };
    }
    // 标准圆弧近似系数：控制点距顶点 = 半径 * 0.552915。
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

/// 生成矩形（可带圆角）轮廓。
///
/// `size` / `radius` 单位为像素；圆角半径会被钳制到半宽/半高。
/// 手工按屏幕坐标（y 向下）顺时针布置 4 个角，每个圆角用 `ARC_STEPS`
/// 段折线近似；`direction < 0` 时反转顶点顺序。
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

    // 屏幕坐标下顺时针（y 向下）；每个圆角额外插入 ARC_STEPS - 1 个点。
    const ARC_STEPS: usize = 6;
    let mut pts: Vec<[f32; 2]> = Vec::with_capacity(8 + ARC_STEPS * 4);
    let corners = [
        // (角点, 圆角圆心, 起始角)，角度从 x 轴正方向逆时针计。
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

/// 生成多角星或正多边形轮廓。
///
/// `points` 为角数（四舍五入并至少为 2），`outer` / `inner` 为外/内半径
/// （像素），`rotation_deg` 为旋转角（度）。`star = true` 时在内外半径之间
/// 交替，顶点数 2N；否则只取外半径，顶点数 N。`direction < 0` 时反转顺序。
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
    // Lottie 的角度 0 指向正上方，故减去 90°（FRAC_PI_2）对齐。
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

/// 按弧长把轮廓重采样为恰好 `k`（至少 2）个点。
///
/// `closed` 控制采样分布：闭合轮廓取 `j / k`（不重复首点），开放轮廓取
/// `j / (k - 1)`（包含首尾端点）。返回新分配的缓冲区。
pub fn resample(contour: &Contour, k: usize, closed: bool) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    resample_into(contour, k, closed, &mut out);
    out
}

/// 与 [`resample`] 相同，但写入可复用的 `out` 缓冲区，且逐段流式前进，
/// 不额外分配「每段累计弧长」表。
///
/// `k` 为每轮廓重采样点数；`closed` 见 [`resample`]。轮廓退化（少于 2 点
/// 或总长为 0）时输出 `k` 个同一点。
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
        // 闭合轮廓均分整圈；开放轮廓把首尾端点也纳入，故分母为 k - 1。
        let target = if closed {
            total * (j as f32 / k as f32)
        } else {
            total * (j as f32 / (k - 1) as f32)
        };
        // 游标只向前推进：目标弧长单调递增，无需每点从头扫描。
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

/// 截取轮廓中从 `start_frac` 开始、长度为 `length_frac` 的一段。
///
/// 两者都是 0..1 的归一化弧长比例；`start_frac` 按 `rem_euclid` 环绕，长度
/// 会被钳制。返回开放轮廓（调用方在填充时自行闭合）。固定采样 48 段，
/// 保证截取结果点数稳定；长度接近 1 时直接返回原轮廓的拷贝。
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
    // 一次遍历得到累计弧长表，避免每个采样点都重新求和。
    let mut cumulative = Vec::with_capacity(segments);
    let mut accumulated = 0.0f32;
    for s in 0..segments {
        accumulated += dist(pts[s], pts[(s + 1) % pts.len()]);
        cumulative.push(accumulated);
    }

    let start_len = total * start;
    let end_len = start_len + total * length;
    // 固定 48 段采样，使 trim 结果的点数与动画时间无关（拓扑稳定）。
    let steps = 48usize;
    let mut out = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let d = start_len + (end_len - start_len) * (i as f32 / steps as f32);
        // 目标弧长按总长环绕，再用二分定位所在段（partition_point）。
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

/// 把 trim 区间拆成最多两个固定槽位，保证区间跨越轮廓起点时拓扑不变。
///
/// `s` / `e` 为 0..1 的起止比例（会排序并钳制），`offset` 为偏移比例。
/// 返回 `[(start, length), ...]`：第一槽是环绕后的首段，第二槽是跨过 0 点
/// 后剩余的部分（可能长度为 0，调用方需保留空槽以稳定计数）。
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
        // 几乎整圈：固定用 (0, 1) + 空槽，避免两段拼接产生接缝。
        return [(0.0, 1.0), (0.0, 0.0)];
    }
    let begin = (a + offset.rem_euclid(1.0)).rem_euclid(1.0);
    let first_len = length.min(1.0 - begin);
    let second_len = (length - first_len).max(0.0);
    [(begin, first_len), (0.0, second_len)]
}
