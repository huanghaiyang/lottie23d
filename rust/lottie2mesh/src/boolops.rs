//! 轮廓布尔运算：为 Lottie 的 Merge Paths（`mm`）提供并集 / 相减 / 相交 / 排除。
//!
//! 模块职责：
//! * 把若干条扁平化轮廓按 AE 的合并模式（1 Merge、2 Add、3 Subtract、
//!   4 Intersect、5 Exclude）合成新的轮廓集合；
//! * 全部计算在转换期完成，播放端拿到的仍是普通 `Contour`，无需任何新数据。
//!
//! 实现思路（「切分 - 分类 - 拼接」）：
//! 1. 把两组轮廓的所有边在交点处切开，切开后任意两条边只在端点接触；
//! 2. 用每段的中点判断它是否落在对方轮廓集合内（奇偶规则），据此决定
//!    保留、丢弃或反向（相减/排除需要反向）；
//! 3. 按端点把保留的边拼回闭合环路，得到结果轮廓。
//!
//! 没有交点时走快速路径：只按包含关系筛选轮廓，避免 O(n·m) 的逐边求交，
//! 让「互不相交的多条字形路径合并」这类常见情况几乎零开销。
//!
//! 已知限制（与 lottie-web 的近似程度一致）：
//! * 边与边共线重叠时不做特殊拆分，中点分类可能落在对方边界上；
//! * 路径自交（单个 `sh` 自身打结）不做自相交重排，交点只按「两组之间」处理；
//! * 合并模式 1 与 2 都按「并集」处理，4 在只有一条路径时是空操作。
//!
//! 单位约定：坐标与 Lottie 像素一致；结果轮廓一律标记为闭合。

use std::collections::HashMap;

use crate::path::Contour;

/// 判定两条边平行的相对容差（用叉积与边长乘积比较，避免尺度影响）。
const PARALLEL_EPS: f32 = 1e-7;
/// 端点容差：交点落在边端点附近时视为「端点接触」，不产生切分。
const ENDPOINT_EPS: f32 = 1e-6;
/// 拼接时坐标键的量化倍率（0.001 像素），用于容忍浮点尾差。
const KEY_SCALE: f64 = 1000.0;

/// 合并模式，取值与 Lottie 的 `mm.mm` 一致。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoolOp {
    /// 1 Merge / 2 Add：并集。
    Union,
    /// 3 Subtract：从第一个轮廓中减去其余轮廓。
    Subtract,
    /// 4 Intersect：交集。
    Intersect,
    /// 5 Exclude：异或（奇偶规则）。
    Exclude,
}

/// 一条闭合环（顶点不重复首点，统一为逆时针）。
#[derive(Clone, Debug)]
struct Ring {
    pts: Vec<[f32; 2]>,
}

impl Ring {
    /// 有符号面积（鞋带公式，逆时针为正）。
    fn area(&self) -> f32 {
        let n = self.pts.len();
        if n < 3 {
            return 0.0;
        }
        let mut sum = 0.0f32;
        for i in 0..n {
            let a = self.pts[i];
            let b = self.pts[(i + 1) % n];
            sum += a[0] * b[1] - b[0] * a[1];
        }
        sum * 0.5
    }

    /// 代表点：取最长边的中点，尽量避开与对方边界重合的顶点。
    fn representative(&self) -> [f32; 2] {
        let n = self.pts.len();
        let mut best = (0.0f32, self.pts[0], self.pts[(1) % n]);
        for i in 0..n {
            let a = self.pts[i];
            let b = self.pts[(i + 1) % n];
            let length = (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2);
            if length > best.0 {
                best = (length, a, b);
            }
        }
        [(best.1[0] + best.2[0]) * 0.5, (best.1[1] + best.2[1]) * 0.5]
    }

    /// 轴对齐包围盒 `[min_x, min_y, max_x, max_y]`。
    fn bounds(&self) -> [f32; 4] {
        let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for p in &self.pts {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
        b
    }
}

/// 一条有向边；`src` 为 0（第一组）或 1（第二组）。
#[derive(Clone, Copy, Debug)]
struct Edge {
    p0: [f32; 2],
    p1: [f32; 2],
    src: u8,
}

/// 对一组轮廓执行布尔合并。
///
/// `Subtract` 以第一个轮廓为基底，依次减去其余轮廓；其余模式对所有轮廓
/// 依次折叠（并集/异或/交均可结合）。返回空列表表示结果没有任何填充区域。
pub fn combine(contours: &[Contour], op: BoolOp) -> Vec<Contour> {
    let mut rings: Vec<Ring> = contours.iter().filter_map(prepare).collect();
    if rings.is_empty() {
        return Vec::new();
    }
    let first = rings.remove(0);
    if rings.is_empty() {
        return vec![to_contour(first)];
    }
    let mut acc = vec![first];
    for next in rings {
        acc = binary(&acc, &[next], op);
        if acc.is_empty() {
            break;
        }
    }
    acc.into_iter().map(to_contour).collect()
}

/// 用一组轮廓（奇偶规则）裁剪主体轮廓，`op` 通常为 `Intersect` 或 `Subtract`。
///
/// 与 [`combine`] 的差别在于：`mask` 的所有轮廓被视为**同一个操作数**，
/// 嵌套环自然按孔洞处理，适合把图层遮罩（可能由多条轮廓组成）一次性套用。
/// `mask` 为空时返回主体原样。
pub fn clip(subject: &[Contour], mask: &[Contour], op: BoolOp) -> Vec<Contour> {
    let a: Vec<Ring> = subject.iter().filter_map(prepare).collect();
    let b: Vec<Ring> = mask.iter().filter_map(prepare).collect();
    if b.is_empty() {
        return subject.to_vec();
    }
    binary(&a, &b, op).into_iter().map(to_contour).collect()
}

/// 把 `Contour` 转成闭合环：去重、判退化、统一为逆时针。
fn prepare(contour: &Contour) -> Option<Ring> {
    let mut pts = contour.pts.clone();
    if pts.len() < 3 {
        return None;
    }
    // 去掉与首点重合的末点，以及连续重复点。
    while pts.len() > 3 && squared_dist(pts[0], *pts.last().unwrap()) < 1e-8 {
        pts.pop();
    }
    let mut cleaned: Vec<[f32; 2]> = Vec::with_capacity(pts.len());
    for p in pts {
        if cleaned
            .last()
            .map(|last| squared_dist(*last, p) < 1e-8)
            .unwrap_or(false)
        {
            continue;
        }
        cleaned.push(p);
    }
    if cleaned.len() < 3 {
        return None;
    }
    let mut ring = Ring { pts: cleaned };
    let area = ring.area();
    if area.abs() < 1e-5 {
        return None;
    }
    // 统一为逆时针，让相减/排除的反向规则与输入绕向无关。
    if area < 0.0 {
        ring.pts.reverse();
    }
    Some(ring)
}

/// 闭合环转回 `Contour`。
fn to_contour(ring: Ring) -> Contour {
    Contour {
        pts: ring.pts,
        closed: true,
    }
}

/// 两组轮廓的二元运算。
fn binary(a: &[Ring], b: &[Ring], op: BoolOp) -> Vec<Ring> {
    if a.is_empty() {
        return match op {
            BoolOp::Union | BoolOp::Exclude => b.to_vec(),
            _ => Vec::new(),
        };
    }
    if b.is_empty() {
        return a.to_vec();
    }
    // 排除（异或）拆成 (A∪B) − (A∩B)：
    // 直接拼接「保留全部边、内部段反向」会在交点处同时出现多条出边，
    // 贪心选边可能沿同一个环自连成完整外轮廓（面积变成并集）。拆成并集
    // 再相减后，每一步的拼接路径唯一，结果就是对称差。
    if op == BoolOp::Exclude {
        let union = binary(a, b, BoolOp::Union);
        let intersection = binary(a, b, BoolOp::Intersect);
        if intersection.is_empty() {
            return union;
        }
        return binary(&union, &intersection, BoolOp::Subtract);
    }
    if !any_crossing(a, b) {
        return combine_without_crossing(a, b, op);
    }
    arrange(a, b, op)
}

/// 两组轮廓之间是否存在真正的边相交（端点接触不算）。
fn any_crossing(a: &[Ring], b: &[Ring]) -> bool {
    let b_bounds: Vec<[f32; 4]> = b.iter().map(Ring::bounds).collect();
    for ra in a {
        let bounds = ra.bounds();
        for (index, rb) in b.iter().enumerate() {
            if !bounds_overlap(&bounds, &b_bounds[index]) {
                continue;
            }
            if rings_cross(ra, rb) {
                return true;
            }
        }
    }
    false
}

/// 两条环的边集是否相交（遇到第一处相交立即返回）。
fn rings_cross(a: &Ring, b: &Ring) -> bool {
    let na = a.pts.len();
    let nb = b.pts.len();
    for i in 0..na {
        let p0 = a.pts[i];
        let p1 = a.pts[(i + 1) % na];
        for j in 0..nb {
            let q0 = b.pts[j];
            let q1 = b.pts[(j + 1) % nb];
            if edge_intersection(p0, p1, q0, q1).is_some() {
                return true;
            }
        }
    }
    false
}

/// 两个包围盒是否重叠。
fn bounds_overlap(a: &[f32; 4], b: &[f32; 4]) -> bool {
    a[2] >= b[0] && b[2] >= a[0] && a[3] >= b[1] && b[3] >= a[1]
}

/// 无交点时的快速合并：只按「代表点是否落在对方集合内」筛选。
fn combine_without_crossing(a: &[Ring], b: &[Ring], op: BoolOp) -> Vec<Ring> {
    match op {
        BoolOp::Union => {
            // 被对方轮廓包住的部分并入对方，避免重复叠放。
            let mut out: Vec<Ring> = a
                .iter()
                .filter(|ring| !point_in_rings(ring.representative(), b))
                .cloned()
                .collect();
            out.extend(
                b.iter()
                    .filter(|ring| !point_in_rings(ring.representative(), a))
                    .cloned(),
            );
            out
        }
        BoolOp::Subtract => {
            // 基底全部保留；被基底包住的减数轮廓变成孔洞。
            let mut out = a.to_vec();
            out.extend(
                b.iter()
                    .filter(|ring| point_in_rings(ring.representative(), a))
                    .cloned(),
            );
            out
        }
        BoolOp::Intersect => {
            // 交集只保留「嵌在对方内部」的那一层轮廓。
            let mut out: Vec<Ring> = a
                .iter()
                .filter(|ring| point_in_rings(ring.representative(), b))
                .cloned()
                .collect();
            out.extend(
                b.iter()
                    .filter(|ring| point_in_rings(ring.representative(), a))
                    .cloned(),
            );
            out
        }
        BoolOp::Exclude => {
            // 奇偶规则下，嵌套轮廓自然形成孔洞，直接全部保留即可。
            let mut out = a.to_vec();
            out.extend(b.iter().cloned());
            out
        }
    }
}

/// 有交点时的完整流程：切分 → 分类 → 拼接。
fn arrange(a: &[Ring], b: &[Ring], op: BoolOp) -> Vec<Ring> {
    let a_edges = collect_edges(a, 0);
    let b_edges = collect_edges(b, 1);
    let mut a_cuts: Vec<Vec<(f32, [f32; 2])>> = vec![Vec::new(); a_edges.len()];
    let mut b_cuts: Vec<Vec<(f32, [f32; 2])>> = vec![Vec::new(); b_edges.len()];

    // 求交并把同一个交点写进两组边，保证拼接时坐标完全一致。
    for (i, ea) in a_edges.iter().enumerate() {
        for (j, eb) in b_edges.iter().enumerate() {
            if let Some((t, u)) = edge_intersection(ea.p0, ea.p1, eb.p0, eb.p1) {
                let point = [
                    ea.p0[0] + (ea.p1[0] - ea.p0[0]) * t,
                    ea.p0[1] + (ea.p1[1] - ea.p0[1]) * t,
                ];
                a_cuts[i].push((t, point));
                b_cuts[j].push((u, point));
            }
        }
    }

    let mut pieces = split_edges(&a_edges, &mut a_cuts);
    pieces.extend(split_edges(&b_edges, &mut b_cuts));

    // 逐段分类：中点落在对方内部则视为「内部段」。
    let mut kept: Vec<Edge> = Vec::with_capacity(pieces.len());
    for edge in pieces {
        let midpoint = [
            (edge.p0[0] + edge.p1[0]) * 0.5,
            (edge.p0[1] + edge.p1[1]) * 0.5,
        ];
        let inside_other = if edge.src == 0 {
            point_in_rings(midpoint, b)
        } else {
            point_in_rings(midpoint, a)
        };
        if let Some(reversed) = keep_flag(op, edge.src, inside_other) {
            kept.push(if reversed {
                Edge {
                    p0: edge.p1,
                    p1: edge.p0,
                    src: edge.src,
                }
            } else {
                edge
            });
        }
    }
    stitch(kept)
}

/// 某一段边是否保留；返回 `Some(true)` 表示需要反向。
fn keep_flag(op: BoolOp, src: u8, inside_other: bool) -> Option<bool> {
    let keep = match op {
        BoolOp::Union => !inside_other,
        BoolOp::Intersect => inside_other,
        BoolOp::Subtract => {
            if src == 0 {
                !inside_other
            } else {
                inside_other
            }
        }
        BoolOp::Exclude => true,
    };
    if !keep {
        return None;
    }
    let reversed = match op {
        BoolOp::Subtract => src == 1,
        BoolOp::Exclude => inside_other,
        _ => false,
    };
    Some(reversed)
}

/// 收集一组轮廓的所有有向边。
fn collect_edges(rings: &[Ring], src: u8) -> Vec<Edge> {
    let mut edges = Vec::new();
    for ring in rings {
        let n = ring.pts.len();
        for i in 0..n {
            let p0 = ring.pts[i];
            let p1 = ring.pts[(i + 1) % n];
            if squared_dist(p0, p1) > 1e-10 {
                edges.push(Edge { p0, p1, src });
            }
        }
    }
    edges
}

/// 按交点参数把每条边切成小段。
fn split_edges(edges: &[Edge], cuts: &mut [Vec<(f32, [f32; 2])>]) -> Vec<Edge> {
    let mut out = Vec::new();
    for (index, edge) in edges.iter().enumerate() {
        let cuts = &mut cuts[index];
        cuts.sort_by(|left, right| {
            left.0
                .partial_cmp(&right.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut previous = edge.p0;
        for &(_, point) in cuts.iter() {
            // 与当前起点重合的切点直接跳过；切点贴近终点时不再单独成段。
            if squared_dist(previous, point) > 1e-8 && squared_dist(point, edge.p1) > 1e-8 {
                out.push(Edge {
                    p0: previous,
                    p1: point,
                    src: edge.src,
                });
            }
            previous = point;
        }
        if squared_dist(previous, edge.p1) > 1e-8 {
            out.push(Edge {
                p0: previous,
                p1: edge.p1,
                src: edge.src,
            });
        }
    }
    out
}

/// 按端点把保留的边拼成闭合环；无法闭合的散段直接丢弃。
fn stitch(edges: Vec<Edge>) -> Vec<Ring> {
    if edges.is_empty() {
        return Vec::new();
    }
    let mut outgoing: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (index, edge) in edges.iter().enumerate() {
        outgoing.entry(key(edge.p0)).or_default().push(index);
    }
    let mut used = vec![false; edges.len()];
    let mut rings = Vec::new();

    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        let start_key = key(edges[start].p0);
        let mut pts: Vec<[f32; 2]> = Vec::new();
        let mut current = start;
        let mut closed = false;
        // 每条边最多走一次，额外留一次容错，防止异常数据导致死循环。
        let mut guard = edges.len() + 1;
        loop {
            used[current] = true;
            pts.push(edges[current].p0);
            let end_key = key(edges[current].p1);
            if end_key == start_key {
                closed = true;
                break;
            }
            let Some(candidates) = outgoing.get_mut(&end_key) else {
                break;
            };
            let mut next = None;
            while let Some(candidate) = candidates.pop() {
                if !used[candidate] {
                    next = Some(candidate);
                    break;
                }
            }
            let Some(candidate) = next else {
                break;
            };
            current = candidate;
            guard -= 1;
            if guard == 0 {
                break;
            }
        }
        if !closed {
            continue;
        }
        let mut cleaned: Vec<[f32; 2]> = Vec::with_capacity(pts.len());
        for p in pts {
            if cleaned
                .last()
                .map(|last| squared_dist(*last, p) < 1e-8)
                .unwrap_or(false)
            {
                continue;
            }
            cleaned.push(p);
        }
        if cleaned.len() < 3 {
            continue;
        }
        let ring = Ring { pts: cleaned };
        if ring.area().abs() < 1e-5 {
            continue;
        }
        rings.push(ring);
    }
    rings
}

/// 坐标量化键；0.001 像素的网格足以吸收浮点尾差。
fn key(p: [f32; 2]) -> (i64, i64) {
    (
        (p[0] as f64 * KEY_SCALE).round() as i64,
        (p[1] as f64 * KEY_SCALE).round() as i64,
    )
}

/// 两条线段的真交点参数 `(t, u)`；平行、端点接触或退化时返回 `None`。
fn edge_intersection(
    p0: [f32; 2],
    p1: [f32; 2],
    q0: [f32; 2],
    q1: [f32; 2],
) -> Option<(f32, f32)> {
    let rx = p1[0] - p0[0];
    let ry = p1[1] - p0[1];
    let sx = q1[0] - q0[0];
    let sy = q1[1] - q0[1];
    let r_len = (rx * rx + ry * ry).sqrt();
    let s_len = (sx * sx + sy * sy).sqrt();
    if r_len < 1e-6 || s_len < 1e-6 {
        return None;
    }
    let denom = rx * sy - ry * sx;
    if denom.abs() <= PARALLEL_EPS * r_len * s_len {
        return None;
    }
    let qpx = q0[0] - p0[0];
    let qpy = q0[1] - p0[1];
    let t = (qpx * sy - qpy * sx) / denom;
    let u = (qpx * ry - qpy * rx) / denom;
    if t <= ENDPOINT_EPS || t >= 1.0 - ENDPOINT_EPS {
        return None;
    }
    if u <= ENDPOINT_EPS || u >= 1.0 - ENDPOINT_EPS {
        return None;
    }
    Some((t, u))
}

/// 奇偶规则：点是否落在一组闭合轮廓（可含孔洞）内部。
fn point_in_rings(point: [f32; 2], rings: &[Ring]) -> bool {
    let mut inside = false;
    for ring in rings {
        if point_in_ring(point, ring) {
            inside = !inside;
        }
    }
    inside
}

/// 射线法判断点是否在单条闭合轮廓内。
fn point_in_ring(point: [f32; 2], ring: &Ring) -> bool {
    let n = ring.pts.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let a = ring.pts[i];
        let b = ring.pts[j];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let x = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
            if point[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// 两点距离的平方。
fn squared_dist(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 轴对齐矩形（逆时针）。
    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Contour {
        Contour {
            pts: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
            closed: true,
        }
    }

    /// 结果按奇偶规则折算后的覆盖面积（嵌套层数为奇数的环视为孔洞）。
    fn effective_area(contours: &[Contour]) -> f32 {
        let rings: Vec<Ring> = contours.iter().filter_map(prepare).collect();
        let mut total = 0.0f32;
        for (index, ring) in rings.iter().enumerate() {
            let point = ring.representative();
            let depth = rings
                .iter()
                .enumerate()
                .filter(|(other, candidate)| {
                    *other != index && point_in_ring(point, candidate)
                })
                .count();
            let area = ring.area().abs();
            total += if depth % 2 == 0 { area } else { -area };
        }
        total
    }

    fn check(op: BoolOp, expected: f32, tolerance: f32) {
        let a = square(0.0, 0.0, 1.0, 1.0);
        let b = square(0.5, 0.5, 1.5, 1.5);
        let result = combine(&[a, b], op);
        let area = effective_area(&result);
        assert!(
            (area - expected).abs() <= tolerance,
            "{op:?} 面积应为 {expected}，实际 {area}（{} 条轮廓）",
            result.len()
        );
    }

    #[test]
    fn overlapping_squares() {
        // 两正方形相交面积 0.25。
        check(BoolOp::Union, 1.75, 0.01);
        check(BoolOp::Subtract, 0.75, 0.01);
        check(BoolOp::Intersect, 0.25, 0.01);
        check(BoolOp::Exclude, 1.5, 0.01);
    }

    #[test]
    fn disjoint_squares() {
        let a = square(0.0, 0.0, 1.0, 1.0);
        let b = square(3.0, 0.0, 4.0, 1.0);
        let union = combine(&[a.clone(), b.clone()], BoolOp::Union);
        assert_eq!(union.len(), 2);
        assert!((effective_area(&union) - 2.0).abs() < 0.01);
        let intersect = combine(&[a, b], BoolOp::Intersect);
        assert!(intersect.is_empty());
    }

    #[test]
    fn contained_subtract_creates_hole() {
        let outer = square(0.0, 0.0, 4.0, 4.0);
        let inner = square(1.0, 1.0, 2.0, 2.0);
        let result = combine(&[outer, inner], BoolOp::Subtract);
        // 内轮廓作为孔洞保留，奇偶规则下面积为 16 − 1。
        assert_eq!(result.len(), 2);
        assert!((effective_area(&result) - 15.0).abs() < 0.01);
    }
}
