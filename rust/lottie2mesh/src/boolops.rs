//! 多边形布尔运算（合并路径与遮罩裁剪）。
//!
//! 模块职责：
//! * 把若干条扁平化轮廓按 AE 的合并模式（1 Merge、2 Add、3 Subtract、
//!   4 Intersect、5 Exclude）合成新的轮廓集合；
//! * 把图层遮罩（可能由多条轮廓组成）一次性裁剪主体轮廓。
//! * 全部计算在转换期完成，播放端拿到的仍是普通 `Contour`，无需任何新数据。
//!
//! 实现委托给 `i_overlay`（基于整数网格的成熟布尔引擎）。此前自研的
//! 「切分—分类—拼接」实现在共线重叠边界上不稳定：shot_01 里的渐变描边与
//! 动画遮罩来自同一条路径，两条描边带边界完全共线，会出现整帧裁剪为空、
//! 结果环跨越远处顶点（表现为图形瞬间消失或变成巨大乱形）。
//!
//! 坐标与 Lottie 像素一致；输入输出统一使用奇偶规则，结果轮廓一律闭合，
//! `i_overlay` 返回的外轮廓为逆时针、孔洞为顺时针，下游三角化按包含关系
//! 重新判定孔洞，因此绕向不会影响填充结果。

use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;

use crate::path::Contour;

/// 判断轮廓是否退化的面积阈值（像素²）。
const MIN_AREA: f32 = 1e-6;

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

impl BoolOp {
    /// 映射到 `i_overlay` 的布尔规则。
    fn rule(self) -> OverlayRule {
        match self {
            BoolOp::Union => OverlayRule::Union,
            BoolOp::Subtract => OverlayRule::Difference,
            BoolOp::Intersect => OverlayRule::Intersect,
            BoolOp::Exclude => OverlayRule::Xor,
        }
    }
}

/// 按**非零环绕**规则消解自相交，把一组轮廓整理成互不重叠的简单环。
///
/// 描边带是「左边界 → 端帽 → 右边界 → 端帽」的单条闭合环；当中心线急转弯、
/// 两侧偏移线的间隔小于半宽时，这条环会自相交。若随后用奇偶规则求交，交叠
/// 区域会被判成孔洞，表现为内弧/转角处出现缺口（`shot_01` 第 172 帧的凹口）。
/// 描边带的左右两侧绕向一致，非零环绕能把交叠区正确填满，因此这里先用它
/// 化简，再做后续裁切。填充轮廓不含自相交，调用方无需对它们使用本函数。
pub fn simplify(contours: &[Contour]) -> Vec<Contour> {
    let rings = to_input(contours);
    if rings.is_empty() {
        return Vec::new();
    }
    let empty: Vec<Vec<[f64; 2]>> = Vec::new();
    let shapes = rings.overlay(&empty, OverlayRule::Union, FillRule::NonZero);
    from_input(flatten(shapes))
}

/// 对一组轮廓执行布尔合并。
///
/// `Subtract` 以第一个轮廓为基底，依次减去其余轮廓；其余模式对所有轮廓
/// 依次折叠（并集/异或/交均可结合）。返回空列表表示结果没有任何填充区域。
pub fn combine(contours: &[Contour], op: BoolOp) -> Vec<Contour> {
    let rings = to_input(contours);
    let Some(first) = rings.first() else {
        return Vec::new();
    };
    let mut acc = vec![first.clone()];
    for next in rings.iter().skip(1) {
        let clip = vec![next.clone()];
        let shapes = acc.overlay(&clip, op.rule(), FillRule::EvenOdd);
        acc = flatten(shapes);
        if acc.is_empty() {
            return Vec::new();
        }
    }
    from_input(acc)
}

/// 用一组轮廓（奇偶规则）裁剪主体轮廓，`op` 通常为 `Intersect` 或 `Subtract`。
///
/// 与 [`combine`] 的差别在于：`mask` 的所有轮廓被视为**同一个操作数**，
/// 嵌套环自然按孔洞处理，适合把图层遮罩（可能由多条轮廓组成）一次性套用。
/// `mask` 为空时返回主体原样。
pub fn clip(subject: &[Contour], mask: &[Contour], op: BoolOp) -> Vec<Contour> {
    let mask_rings = to_input(mask);
    if mask_rings.is_empty() {
        return subject.to_vec();
    }
    let subject_rings = to_input(subject);
    if subject_rings.is_empty() {
        return Vec::new();
    }
    let shapes = subject_rings.overlay(&mask_rings, op.rule(), FillRule::EvenOdd);
    from_input(flatten(shapes))
}

/// 把 `Contour` 转成布尔引擎输入：去掉连续重复点，过滤退化轮廓。
fn to_input(contours: &[Contour]) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::with_capacity(contours.len());
    for contour in contours {
        let mut pts: Vec<[f64; 2]> = Vec::with_capacity(contour.pts.len());
        for p in &contour.pts {
            let q = [p[0] as f64, p[1] as f64];
            if let Some(last) = pts.last() {
                if (last[0] - q[0]).abs() + (last[1] - q[1]).abs() < 1e-9 {
                    continue;
                }
            }
            pts.push(q);
        }
        // 闭合轮廓末尾重复首点时去掉，引擎会自动闭合。
        while pts.len() > 3 {
            let first = pts[0];
            let last = pts[pts.len() - 1];
            if (first[0] - last[0]).abs() + (first[1] - last[1]).abs() < 1e-9 {
                pts.pop();
            } else {
                break;
            }
        }
        if pts.len() >= 3 && signed_area(&pts).abs() > 1e-9 {
            out.push(pts);
        }
    }
    out
}

/// 把布尔引擎输出的形状压平成「每条轮廓一项」的列表（外轮廓 + 孔洞都保留）。
fn flatten(shapes: Vec<Vec<Vec<[f64; 2]>>>) -> Vec<Vec<[f64; 2]>> {
    shapes.into_iter().flatten().collect()
}

/// 把布尔引擎的 f64 轮廓转回 f32 的 `Contour`，并丢弃退化环。
fn from_input(rings: Vec<Vec<[f64; 2]>>) -> Vec<Contour> {
    let mut out = Vec::with_capacity(rings.len());
    for ring in rings {
        if ring.len() < 3 {
            continue;
        }
        let pts: Vec<[f32; 2]> = ring.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
        if signed_area_f32(&pts).abs() < MIN_AREA {
            continue;
        }
        out.push(Contour { pts, closed: true });
    }
    out
}

/// f64 轮廓的有符号面积（鞋带公式，逆时针为正）。
fn signed_area(pts: &[[f64; 2]]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum * 0.5
}

/// f32 轮廓的有符号面积（用于过滤退化环）。
fn signed_area_f32(pts: &[[f32; 2]]) -> f32 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum * 0.5
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

    /// 结果的有符号面积之和：外轮廓逆时针为正、孔洞顺时针为负。
    fn effective_area(contours: &[Contour]) -> f32 {
        contours
            .iter()
            .map(|c| signed_area_f32(&c.pts).abs() * if signed_area_f32(&c.pts) >= 0.0 { 1.0 } else { -1.0 })
            .sum()
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
        // 内轮廓作为孔洞保留，面积为 16 − 1。
        assert_eq!(result.len(), 2);
        assert!((effective_area(&result) - 15.0).abs() < 0.01);
    }

    #[test]
    fn simplify_keeps_ring_hole() {
        // 描边带的闭合情形：外环逆时针、内环顺时针，化简后必须保留孔洞。
        let outer = square(0.0, 0.0, 4.0, 4.0);
        let mut inner = square(1.0, 1.0, 2.0, 2.0);
        inner.pts.reverse();
        let result = simplify(&[outer, inner]);
        assert_eq!(result.len(), 2, "外环与内环都应保留");
        assert!((effective_area(&result) - 15.0).abs() < 0.01);
    }

    #[test]
    fn coincident_edges_clip() {
        // 两个完全重合的矩形：交集应完整保留，不能因为共线边界丢面积。
        let a = square(0.0, 0.0, 2.0, 2.0);
        let b = square(0.0, 0.0, 2.0, 2.0);
        let result = clip(&[a], &[b], BoolOp::Intersect);
        assert!(!result.is_empty());
        assert!((effective_area(&result) - 4.0).abs() < 0.01);
    }
}
