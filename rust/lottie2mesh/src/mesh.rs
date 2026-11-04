//! 网格构建：填充三角化、描边带生成，并沿 Z 挤出为带前后盖与侧墙的闭合体积。
//!
//! 模块职责：
//! * 把 `shape::Drawable` 的轮廓槽位重采样到固定点数，生成 2D 顶点；
//! * 填充用 earcut 三角化（含孔洞分组），并做 Delaunay 边翻转改善三角形质量；
//! * 描边生成左右偏移的 ribbon；
//! * 用边界边（只被一个三角形引用的边）生成侧墙，前后盖按 `stride` 复制；
//! * 支持按姿态桶（不同采样计划）逐帧重写顶点。
//!
//! 关键数据流：
//! ```text
//! drawable_slots → SlotPlan(k, closed) → resample 采样点
//!   → tessellate_fill / ribbon → surface indices
//!   → extrude (前盖 + 后盖 + 侧墙) → MeshSurface
//! ```
//!
//! 单位约定：坐标、半厚度 `half_thickness`、圆角 `edge_radius` 均为 Lottie
//! 像素；顶点为 `[x, y, z]` 的 f32，索引为 u32。每个轮廓点沿挤出剖面生成
//! `stride` 个顶点：尖边 `stride = 2`（上下盖），圆角时
//! `stride = 2 * ROUND_SEGMENTS + 2 = 8`。

use std::borrow::Cow;
use std::collections::HashMap;

use crate::path::{self, Contour};
use crate::shape::{DrawKind, Drawable};

/// 每个 1/4 圆角剖面的折线段数；决定圆角插值点数。
const ROUND_SEGMENTS: usize = 3;

/// 每条轮廓重采样点数 `k` 的下限与安全上限。
///
/// `k` 直接由轮廓周长和 `curve_spacing` 算出，因此采样间距对长轮廓也能真实
/// 生效。下限取 3 而不是 6：小轮廓（周长十几像素的圆点、灯泡）在间距调大时
/// 仍能按 `ceil(length / spacing)` 真实降采样，避免所有档位都被抬到同一点数、
/// 让滑块看起来失效；3 也是一条闭合轮廓能围成三角形的最小点数，再低会让
/// 填充三角化退化。8192 只用于防止极端路径（数万像素周长）撑爆显存的兜底，
/// 正常画布尺寸下的轮廓不会触及。整体内存还会被 8M float 预算进一步约束。
const MIN_K: usize = 3;
const MAX_K: usize = 8192;
/// 每个网格允许的拓扑桶上限（姿态锚点 + 折叠触发 + 拓扑变化共用）。
///
/// 烘焙时按采样区间等距设置“姿态锚点”，并在检测到前盖三角形明显反绕/
/// 重叠时额外新建一套三角化，避免拿相隔很远的姿态去套当前形变（剧烈形变
/// 会让三角形反绕、重叠，填充出现裂缝）。上限 24 能覆盖常见形变，又不会
/// 让索引内存与播放端索引缓冲无限膨胀。
pub const MAX_BUCKETS: usize = 24;

/// 单个轮廓槽位的采样计划：`k` 为每轮廓重采样点数，`closed` 决定首尾是否相连。
#[derive(Clone, Debug)]
pub struct SlotPlan {
    pub k: usize,
    pub closed: bool,
    /// 复合填充中的孔洞轮廓；其法线偏移方向会翻转（朝实体内部）。
    pub hole: bool,
}

/// 一张网格的参考几何与固定拓扑。
pub struct MeshSurface {
    /// 各轮廓槽位的采样计划。
    pub slots: Vec<SlotPlan>,
    /// 2D 采样点总数（所有槽位 `k` 之和）。
    pub point_count: usize,
    /// 每个轮廓点沿挤出剖面生成的顶点数（尖边 2，圆角 8）。
    pub stride: usize,
    /// 三角化 + 挤出后的索引缓冲。
    pub indices: Vec<u32>,
    /// 参考帧的展开顶点坐标，布局为 `[x, y, z]`，长度 `point_count * stride * 3`。
    pub ref_positions: Vec<f32>,
    /// 逐顶点 RGBA（0..1），长度 `point_count * stride * 4`；仅渐变绘制项有值。
    /// 颜色按参考帧顶点的 2D 位置采样，动画期间保持不变以节省内存。
    pub ref_colors: Option<Vec<f32>>,
}

/// 每个轮廓点生成的顶点数：尖边 2 个，圆角为 `2 * ROUND_SEGMENTS + 2`。
fn profile_stride(radius: f32) -> usize {
    if radius > 1e-4 {
        // 上下各一个 1/4 圆角 + 上下盖边界点，共 8 个。
        2 * ROUND_SEGMENTS + 2
    } else {
        2
    }
}

/// 计算某圈剖面相对轮廓点的法线偏移量与 Z 坐标。
///
/// 返回 `(offset, z)`：`offset` 沿轮廓内法线方向（像素），`z` 为挤出深度
/// （像素，范围约 `[-half, half]`）。`radius <= 0` 时退化为上下两个盖面。
fn ring_profile(index: usize, radius: f32, half: f32) -> (f32, f32) {
    if radius <= 1e-4 {
        return (0.0, if index == 0 { half } else { -half });
    }
    if index <= ROUND_SEGMENTS {
        // 上半 1/4 圆角：从侧墙过渡到上盖边界。
        let theta = (1.0 - index as f32 / ROUND_SEGMENTS as f32)
            * std::f32::consts::FRAC_PI_2;
        (
            radius * (1.0 - theta.cos()),
            (half - radius) + radius * theta.sin(),
        )
    } else {
        // 下半 1/4 圆角：从下盖边界过渡到侧墙。
        let local = index - (ROUND_SEGMENTS + 1);
        let theta = local as f32 / ROUND_SEGMENTS as f32 * std::f32::consts::FRAC_PI_2;
        (
            radius * (1.0 - theta.cos()),
            -(half - radius) - radius * theta.sin(),
        )
    }
}

/// 基于 drawable 的槽位构建参考网格与固定拓扑。
///
/// `half_thickness` 为挤出半厚（像素），`edge_radius` 为圆角半径（像素，
/// 仅填充生效并钳制到半厚）；`curve_spacing` 为轮廓采样间距（像素/点，
/// 越小越平滑）。无有效轮廓或三角化失败时返回 `None`。
pub fn build_surface(
    drawable: &Drawable,
    half_thickness: f32,
    edge_radius: f32,
    curve_spacing: f32,
) -> Option<MeshSurface> {
    let slots = crate::shape::drawable_slots(drawable);
    if slots.is_empty() {
        return None;
    }
    let plans = plan_slots(&slots, curve_spacing);
    build_surface_from_slots(drawable, &slots, plans, half_thickness, edge_radius)
}

/// 单条轮廓的自然重采样密度：约每 `spacing` 像素一个点；并钳制到
/// `[MIN_K, MAX_K]`，近乎零长度时返回 `MIN_K`（占位用的最小闭合轮廓）。
///
/// `spacing` 由调用方传入（转换选项 `curve_spacing`，默认 3.0），下限 1 像素、
/// 上限 8 像素由 [`crate::compile::RawOptions`] 统一钳制；`MAX_K = 8192` 仅作
/// 极端路径的显存兜底，正常轮廓的采样密度由 `spacing` 决定。
pub fn natural_k(contour: &Contour, spacing: f32) -> usize {
    let length = contour.length();
    if length < 1e-4 {
        MIN_K
    } else {
        ((length / spacing).ceil() as usize).clamp(MIN_K, MAX_K)
    }
}

/// 为每个槽位生成采样计划（此时孔洞标记未知，统一置 `false`）。
pub fn plan_slots(slots: &[(Cow<'_, Contour>, bool, bool)], spacing: f32) -> Vec<SlotPlan> {
    slots
        .iter()
        .map(|(contour, closed, _)| SlotPlan {
            k: natural_k(contour, spacing),
            closed: *closed,
            hole: false,
        })
        .collect()
}

/// 用显式槽位和采样计划构建网格（烘焙时发现新拓扑桶会走这条路径）。
///
/// `plans` 会被就地补全 `hole` 标记。仅填充使用 `edge_radius`；描边固定用
/// 尖边剖面。采样点为空或 surface 三角化失败时返回 `None`。
pub fn build_surface_from_slots(
    drawable: &Drawable,
    slots: &[(Cow<'_, Contour>, bool, bool)],
    mut plans: Vec<SlotPlan>,
    half_thickness: f32,
    edge_radius: f32,
) -> Option<MeshSurface> {
    if slots.is_empty() || plans.len() != slots.len() {
        return None;
    }

    // 只有填充做圆角；描边 ribbon 保持尖边剖面。半径不得超过半厚。
    let radius = if drawable.kind == DrawKind::Fill {
        edge_radius.clamp(0.0, half_thickness)
    } else {
        0.0
    };
    let stride = profile_stride(radius);

    let points = build_points(drawable, slots, &plans)?;
    if drawable.kind == DrawKind::Fill && radius > 0.0 {
        // 圆角挤出需要知道哪些轮廓是孔洞，才能决定法线偏移方向。
        let holes = classify_holes(&points, &plans);
        for (plan, hole) in plans.iter_mut().zip(holes) {
            plan.hole = hole;
        }
    }

    let surface_indices = match drawable.kind {
        DrawKind::Fill => tessellate_fill(&points, &plans),
        DrawKind::Stroke => build_ribbon_indices(&plans),
    };
    if surface_indices.is_empty() {
        return None;
    }

    let point_count = points.len();
    // 2D 表面索引 → 挤出后的 3D 索引（前盖/后盖/侧墙一并生成）。
    let indices = extrude(&surface_indices, point_count, stride);
    let mut normals = Vec::new();
    if radius > 0.0 {
        normals.reserve(point_count);
        fill_normals(&points, &plans, &mut normals);
    }
    let ref_positions = expand_positions(&points, &normals, stride, radius, half_thickness);
    // 渐变颜色按参考帧顶点位置采样：逐顶点写入 RGBA（alpha 来自透明度 stop）。
    let ref_colors = drawable.gradient.as_ref().map(|gradient| {
        let mut colors = Vec::with_capacity(ref_positions.len() * 4 / 3);
        for vertex in ref_positions.chunks_exact(3) {
            let point = [vertex[0], vertex[1]];
            let rgb = gradient.color_at(point);
            colors.extend_from_slice(&[rgb[0], rgb[1], rgb[2], gradient.alpha_at(point)]);
        }
        colors
    });

    Some(MeshSurface {
        slots: plans,
        point_count,
        stride,
        indices,
        ref_positions,
        ref_colors,
    })
}

/// 逐帧烘焙时复用的缓冲区，避免每帧重新分配。
///
/// 所有网格共享一个实例；`sampled` 按槽位复用，`points` / `normals`
/// 保存当前帧中间结果。
#[derive(Default)]
pub struct FrameScratch {
    sampled: Vec<Vec<[f32; 2]>>,
    points: Vec<[f32; 2]>,
    normals: Vec<[f32; 2]>,
}

/// 为另一帧重建顶点位置，保持参考拓扑并直接写入目标轨迹切片。
///
/// `out` 的长度必须至少为 `point_count * stride * 3`；数据不可写或几何
/// 退化时返回 `false`，此时调用方应回滚已写入的长度。
pub fn write_frame(
    drawable: &Drawable,
    plans: &[SlotPlan],
    half_thickness: f32,
    edge_radius: f32,
    out: &mut [f32],
    scratch: &mut FrameScratch,
) -> bool {
    let slots = crate::shape::drawable_slots(drawable);
    write_frame_with_slots(drawable, &slots, plans, half_thickness, edge_radius, out, scratch)
}

/// 与 [`write_frame`] 相同，但复用调用方已算好的槽位（编译器选桶时已
/// 取得），避免重复展开 trim 槽位。
pub fn write_frame_with_slots(
    drawable: &Drawable,
    slots: &[(Cow<'_, Contour>, bool, bool)],
    plans: &[SlotPlan],
    half_thickness: f32,
    edge_radius: f32,
    out: &mut [f32],
    scratch: &mut FrameScratch,
) -> bool {
    if slots.is_empty() {
        return false;
    }
    let radius = if drawable.kind == DrawKind::Fill {
        edge_radius.clamp(0.0, half_thickness)
    } else {
        0.0
    };
    let stride = profile_stride(radius);
    if scratch.sampled.len() < plans.len() {
        scratch.sampled.resize_with(plans.len(), Vec::new);
    }
    // 按桶计划重采样；槽位缺失时用单点退化轮廓占位，保证点数不变。
    for (index, plan) in plans.iter().enumerate() {
        let buffer = &mut scratch.sampled[index];
        match slots.get(index) {
            Some((contour, closed, _)) => {
                path::resample_into(contour, plan.k, *closed && plan.closed, buffer);
            }
            None => {
                let origin = slots
                    .last()
                    .and_then(|(c, _, _)| c.pts.first().copied())
                    .unwrap_or([0.0, 0.0]);
                let degenerate = Contour {
                    pts: vec![origin],
                    closed: false,
                };
                path::resample_into(&degenerate, plan.k, false, buffer);
            }
        }
    }

    scratch.points.clear();
    match drawable.kind {
        DrawKind::Fill => {
            for chunk in &scratch.sampled[..plans.len()] {
                scratch.points.extend_from_slice(chunk);
            }
        }
        DrawKind::Stroke => {
            let samples = &scratch.sampled[..plans.len()];
            if !ribbon_points(drawable, plans, samples, &mut scratch.points) {
                return false;
            }
        }
    }

    if radius > 0.0 {
        scratch.normals.clear();
        fill_normals(&scratch.points, plans, &mut scratch.normals);
    }
    let points = &scratch.points;
    // 目标切片容量不够时放弃写入，交由上层截断。
    if points.len() * stride * 3 > out.len() {
        return false;
    }
    // 每个采样点展开成 stride 个顶点：2D 坐标沿法线偏移，Z 取剖面值。
    for (index, p) in points.iter().enumerate() {
        let normal = scratch.normals.get(index).copied().unwrap_or([0.0, 0.0]);
        for ring in 0..stride {
            let (offset, z) = ring_profile(ring, radius, half_thickness);
            let base = (index * stride + ring) * 3;
            out[base] = p[0] + normal[0] * offset;
            out[base + 1] = p[1] + normal[1] * offset;
            out[base + 2] = z;
        }
    }
    true
}

/// 重采样全部槽位并拼成 2D 点缓冲（填充直接拼接，描边先转 ribbon）。
fn build_points(
    drawable: &Drawable,
    slots: &[(Cow<'_, Contour>, bool, bool)],
    plans: &[SlotPlan],
) -> Option<Vec<[f32; 2]>> {
    let mut sampled: Vec<Vec<[f32; 2]>> = Vec::with_capacity(slots.len());
    for (index, (contour, closed, _)) in slots.iter().enumerate() {
        let k = plans[index].k;
        // 填充由三角化器负责闭合；trim 路径保持开放采样，弧长才正确。
        sampled.push(path::resample(contour, k, *closed));
    }
    match drawable.kind {
        DrawKind::Fill => Some(concat_points(&sampled)),
        DrawKind::Stroke => {
            let mut out = Vec::new();
            if ribbon_points(drawable, plans, &sampled, &mut out) {
                Some(out)
            } else {
                None
            }
        }
    }
}

/// 把多条轮廓的采样点顺序拼接为一个缓冲。
fn concat_points(chunks: &[Vec<[f32; 2]>]) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend_from_slice(chunk);
    }
    out
}

/// 把每条轮廓的采样点展开为描边 ribbon：每个点生成左右两个顶点。
///
/// 宽度取 `drawable.width / 2`（像素）；沿相邻点切线求法线，并按斜接角
/// 放大偏移，最多到 `miter` 倍数。宽度过小（< 0.05）时返回 `false`。
fn ribbon_points(
    drawable: &Drawable,
    plans: &[SlotPlan],
    samples: &[Vec<[f32; 2]>],
    out: &mut Vec<[f32; 2]>,
) -> bool {
    let half = drawable.width * 0.5;
    if half.abs() < 0.05 {
        return false;
    }
    for (index, points) in samples.iter().enumerate() {
        let is_closed = plans.get(index).map(|plan| plan.closed).unwrap_or(false);
        let n = points.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let prev = if i == 0 {
                if is_closed { n - 1 } else { 0 }
            } else {
                i - 1
            };
            let next = if i == n - 1 {
                if is_closed { 0 } else { n - 1 }
            } else {
                i + 1
            };
            let a = points[prev];
            let b = points[next];
            let mut tx = b[0] - a[0];
            let mut ty = b[1] - a[1];
            let len = (tx * tx + ty * ty).sqrt();
            if len < 1e-6 {
                if i > 0 {
                    let prev_p = points[i - 1];
                    tx = points[i][0] - prev_p[0];
                    ty = points[i][1] - prev_p[1];
                } else {
                    tx = 1.0;
                    ty = 0.0;
                }
                let l = (tx * tx + ty * ty).sqrt().max(1e-6);
                tx /= l;
                ty /= l;
            } else {
                tx /= len;
                ty /= len;
            }
            let nx = -ty;
            let ny = tx;
            // 斜接因子：相邻两段法线夹角的余弦，钳制到 0.2 防止尖刺无限长。
            let dot = if i == 0 && !is_closed {
                1.0
            } else if i == n - 1 && !is_closed {
                1.0
            } else {
                let p0 = points[i];
                let d0x = p0[0] - a[0];
                let d0y = p0[1] - a[1];
                let l0 = (d0x * d0x + d0y * d0y).sqrt().max(1e-6);
                let d1x = b[0] - p0[0];
                let d1y = b[1] - p0[1];
                let l1 = (d1x * d1x + d1y * d1y).sqrt().max(1e-6);
                let n0x = -d0y / l0;
                let n0y = d0x / l0;
                let n1x = -d1y / l1;
                let n1y = d1x / l1;
                (n0x * n1x + n0y * n1y).clamp(0.2, 1.0)
            };
            // 斜接长度受样式 miter 限制，避免锐角处过长。
            let miter = (half / dot).min(half * drawable.miter.max(1.0));
            out.push([points[i][0] + nx * miter, points[i][1] + ny * miter]);
            out.push([points[i][0] - nx * miter, points[i][1] - ny * miter]);
        }
    }
    !out.is_empty()
}

/// 为 ribbon 顶点生成表面三角形：每段连接左右两侧，形成两个三角形。
fn build_ribbon_indices(plans: &[SlotPlan]) -> Vec<u32> {
    let mut indices = Vec::new();
    let mut base = 0u32;
    for plan in plans {
        let k = plan.k;
        let segments = if plan.closed { k } else { k - 1 };
        for i in 0..segments {
            let next = (i + 1) % k;
            let l0 = base + (i * 2) as u32;
            let r0 = l0 + 1;
            let l1 = base + (next * 2) as u32;
            let r1 = l1 + 1;
            indices.extend_from_slice(&[l0, r0, r1, l0, r1, l1]);
        }
        base += (k * 2) as u32;
    }
    indices
}

/// 用 earcut 三角化填充：先按包含关系把轮廓分成「外轮廓 + 孔洞」组。
///
/// 输入 `points` 是所有槽位采样点的顺序拼接（每槽 `k` 点），返回值索引
/// 指向该缓冲。面积过小、退化或 earcut 失败的组会被跳过。
fn tessellate_fill(points: &[[f32; 2]], plans: &[SlotPlan]) -> Vec<u32> {
    // 把拼接点缓冲重新拆回各轮廓，并按包含关系分组孔洞。
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut offset = 0usize;
    for plan in plans {
        let k = plan.k;
        if offset + k > points.len() {
            break;
        }
        contours.push(points[offset..offset + k].to_vec());
        offset += k;
    }

    // 按面积绝对值从大到小排序，保证外轮廓先于其孔洞被处理。
    let mut order: Vec<usize> = (0..contours.len()).collect();
    order.sort_by(|&a, &b| {
        area(&contours[b])
            .abs()
            .partial_cmp(&area(&contours[a]).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut outer_of: Vec<Option<usize>> = vec![None; contours.len()];
    for &index in &order {
        // 找一个能包含它的、面积更大的外轮廓组，取面积最小的那个作为直接父级。
        if contours[index].len() < 3 || area(&contours[index]).abs() < 1e-3 {
            continue;
        }
        let representative = representative_point(&contours[index]);
        let mut best: Option<usize> = None;
        for (group_index, group) in groups.iter().enumerate() {
            let outer = group[0];
            if area(&contours[outer]).abs() <= area(&contours[index]).abs() {
                continue;
            }
            if point_in_polygon(representative, &contours[outer]) {
                if best.is_none()
                    || area(&contours[outer]).abs()
                        < area(&contours[groups[best.unwrap()][0]]).abs()
                {
                    best = Some(group_index);
                }
            }
        }
        match best {
            Some(group_index) => {
                groups[group_index].push(index);
                outer_of[index] = Some(groups[group_index][0]);
            }
            None => {
                groups.push(vec![index]);
                outer_of[index] = Some(index);
            }
        }
    }

    let offsets = contour_offsets(plans);
    // 每个全局采样点所属的轮廓槽位；边翻转只允许发生在同一轮廓内部。
    let mut contour_id = vec![0u32; points.len()];
    for (index, plan) in plans.iter().enumerate() {
        let start = offsets[index];
        let end = (start + plan.k).min(points.len());
        for id in &mut contour_id[start..end] {
            *id = index as u32;
        }
    }
    let mut indices = Vec::new();
    // 只处理作为组根的外轮廓；每组把外轮廓与孔洞顶点拼成 earcut 输入。
    for group in groups {
        let outer = group[0];
        if outer_of[outer] != Some(outer) {
            continue;
        }
        let mut data: Vec<f64> = Vec::new();
        let mut local_to_global: Vec<u32> = Vec::new();
        // earcut 的 holes 参数记录每个孔洞在 data 中的顶点起始下标。
        let mut holes: Vec<usize> = Vec::new();
        let mut local_vertex = 0usize;
        for (slot, contour_index) in group.iter().enumerate() {
            if slot > 0 {
                holes.push(local_vertex);
            }
            let contour = &contours[*contour_index];
            let base = offsets[*contour_index] as u32;
            for (offset_in_contour, p) in contour.iter().enumerate() {
                data.push(p[0] as f64);
                data.push(p[1] as f64);
                local_to_global.push(base + offset_in_contour as u32);
                local_vertex += 1;
            }
        }
        // earcut 只接受扁平的 f64 坐标数组，维度 2；结果索引再映射回全局缓冲。
        let triangles = match earcutr::earcut(&data, &holes, 2) {
            Ok(triangles) => triangles,
            Err(_) => continue,
        };
        for idx in triangles {
            if let Some(global) = local_to_global.get(idx) {
                indices.push(*global);
            }
        }
    }
    // earcut 只保证覆盖，不保证三角形质量：它常产生横跨多个凹口的细长三角形。
    // 形变时这些长边极易反绕，填充会出现折叠/裂缝。这里用 Delaunay 边翻转
    // 把三角形改造成更“局部”的形状（只翻同一条轮廓内部的公共边，不改边界）。
    reduce_long_triangles(points, &contour_id, &mut indices);
    indices
}

/// 对 earcut 结果做若干轮 Delaunay 边翻转（Lawson flip），减少横跨凹口的长三角形。
///
/// 只处理被两个三角形共享、且两端点属于同一条轮廓的内部边；边界边和跨越
/// 孔洞/不同轮廓的连接边保持不变，因此不会破坏外轮廓或孔洞。每轮把一条
/// 公共边 `uv` 换成 `ab`（四顶点严格凸且满足空圆条件时），并保持三角形
/// 绕向一致，保证挤出后的前后盖与侧墙仍然合法。
fn reduce_long_triangles(points: &[[f32; 2]], contour_id: &[u32], indices: &mut [u32]) {
    let triangle_count = indices.len() / 3;
    if triangle_count < 2 {
        return;
    }
    // 统一为逆时针绕向，后续空圆测试与翻转都依赖这一约定。
    for triangle in 0..triangle_count {
        let base = triangle * 3;
        let a = points[indices[base] as usize];
        let b = points[indices[base + 1] as usize];
        let c = points[indices[base + 2] as usize];
        if orient(a, b, c) < 0.0 {
            indices.swap(base + 1, base + 2);
        }
    }

    // 每轮最多翻转一遍；同一轮内被改写的三角形不再参与，避免状态互相干扰。
    const MAX_PASSES: usize = 6;
    for _ in 0..MAX_PASSES {
        // (无向边) → 两侧三角形的 (下标, 对顶点, 该三角形是否沿键的正方向使用此边)。
        let mut edges: HashMap<(u32, u32), Vec<(usize, u32, bool)>> = HashMap::new();
        for triangle in 0..triangle_count {
            let base = triangle * 3;
            let tri = [indices[base], indices[base + 1], indices[base + 2]];
            for corner in 0..3 {
                let u = tri[corner];
                let v = tri[(corner + 1) % 3];
                if u == v {
                    continue;
                }
                let key = if u < v { (u, v) } else { (v, u) };
                let forward = u < v;
                edges
                    .entry(key)
                    .or_default()
                    .push((triangle, tri[(corner + 2) % 3], forward));
            }
        }

        let mut touched = vec![false; triangle_count];
        let mut flipped = 0usize;
        for (&(u, v), owners) in edges.iter() {
            // 只翻转内部边：恰好两个三角形共享，且四顶点都在同一条轮廓上。
            if owners.len() != 2 {
                continue;
            }
            // 两个三角形应以相反方向共享此边；否则说明拓扑已异常，跳过。
            if owners[0].2 == owners[1].2 {
                continue;
            }
            let forward = if owners[0].2 { owners[0] } else { owners[1] };
            let backward = if owners[0].2 { owners[1] } else { owners[0] };
            let (t0, a) = (forward.0, forward.1);
            let (t1, b) = (backward.0, backward.1);
            if touched[t0] || touched[t1] {
                continue;
            }
            let (u, v, a, b) = (u as usize, v as usize, a as usize, b as usize);
            if contour_id[u] != contour_id[v]
                || contour_id[a] != contour_id[u]
                || contour_id[b] != contour_id[u]
            {
                continue;
            }
            let pu = points[u];
            let pv = points[v];
            let pa = points[a];
            let pb = points[b];
            // 四边形必须严格凸，否则新对角线会跑到形状外面。
            if orient(pu, pv, pa) * orient(pu, pv, pb) >= 0.0
                || orient(pa, pb, pu) * orient(pa, pb, pv) >= 0.0
            {
                continue;
            }
            // 空圆测试：b 落在三角形 (u, v, a) 的外接圆内则翻转。
            if !in_circle(pu, pv, pa, pb) {
                continue;
            }
            let base0 = t0 * 3;
            indices[base0] = a as u32;
            indices[base0 + 1] = u as u32;
            indices[base0 + 2] = b as u32;
            let base1 = t1 * 3;
            indices[base1] = b as u32;
            indices[base1 + 1] = v as u32;
            indices[base1 + 2] = a as u32;
            touched[t0] = true;
            touched[t1] = true;
            flipped += 1;
        }
        if flipped == 0 {
            break;
        }
    }
}

/// 二维叉积（带符号面积的两倍）；用于判断绕向与凸性。
fn orient(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f64 {
    let (ax, ay) = (a[0] as f64, a[1] as f64);
    let (bx, by) = (b[0] as f64, b[1] as f64);
    let (cx, cy) = (c[0] as f64, c[1] as f64);
    (bx - ax) * (cy - ay) - (by - ay) * (cx - ax)
}

/// 空圆测试：`d` 是否落在逆时针三角形 `abc` 的外接圆内。
fn in_circle(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> bool {
    let (adx, ady) = ((a[0] - d[0]) as f64, (a[1] - d[1]) as f64);
    let (bdx, bdy) = ((b[0] - d[0]) as f64, (b[1] - d[1]) as f64);
    let (cdx, cdy) = ((c[0] - d[0]) as f64, (c[1] - d[1]) as f64);
    let ab = adx * bdy - ady * bdx;
    let bc = bdx * cdy - bdy * cdx;
    let ca = cdx * ady - cdy * adx;
    let det = ab * (cdx * cdx + cdy * cdy) + bc * (adx * adx + ady * ady)
        + ca * (bdx * bdx + bdy * bdy);
    det > 0.0
}

/// 计算每个槽位在拼接点缓冲中的起始下标。
fn contour_offsets(plans: &[SlotPlan]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(plans.len());
    let mut offset = 0usize;
    for plan in plans {
        offsets.push(offset);
        offset += plan.k;
    }
    offsets
}

/// 鞋带公式求多边形有符号面积；符号反映绕向。
fn area(contour: &[[f32; 2]]) -> f32 {
    let mut sum = 0.0;
    for i in 0..contour.len() {
        let a = contour[i];
        let b = contour[(i + 1) % contour.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum * 0.5
}

/// 取多边形内的一点作为包含性测试代表点：优先质心，质心在外部时退回中点。
fn representative_point(contour: &[[f32; 2]]) -> [f32; 2] {
    let mut cx = 0.0;
    let mut cy = 0.0;
    for p in contour {
        cx += p[0];
        cy += p[1];
    }
    let n = contour.len() as f32;
    let center = [cx / n, cy / n];
    if point_in_polygon(center, contour) {
        center
    } else {
        let a = contour[0];
        let b = contour[1.min(contour.len() - 1)];
        [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]
    }
}

/// 射线法点在多边形内测试（奇偶规则），用于孔洞识别。
fn point_in_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[j];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let t = (point[1] - a[1]) / (b[1] - a[1]);
            let x = a[0] + t * (b[0] - a[0]);
            if point[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// 挤出平面三角面：前盖、后盖与边界侧墙。
///
/// `surface` 是 2D 表面索引（每 3 个一组），`point_count` 为轮廓点数，
/// `stride` 为每点剖面顶点数。前盖取每点第 0 圈（`z = +half`），后盖取
/// 第 `stride - 1` 圈（`z = -half`），并反转绕向；侧墙只对「只被一个
/// 三角形引用」的边界边生成，按相邻两圈连成四边形。
fn extrude(surface: &[u32], point_count: usize, stride: usize) -> Vec<u32> {
    let mut indices = Vec::with_capacity(surface.len() * 2 + 64);
    let last = stride - 1;

    // 前盖：2D 三角索引映射到第 0 圈顶点。
    for tri in surface.chunks_exact(3) {
        indices.extend_from_slice(&[
            (tri[0] as usize * stride) as u32,
            (tri[1] as usize * stride) as u32,
            (tri[2] as usize * stride) as u32,
        ]);
    }
    // 后盖：映射到最后一圈顶点，并反转绕向以保持朝外。
    for tri in surface.chunks_exact(3) {
        indices.extend_from_slice(&[
            (tri[2] as usize * stride + last) as u32,
            (tri[1] as usize * stride + last) as u32,
            (tri[0] as usize * stride + last) as u32,
        ]);
    }

    // 边界边：把每条无向边计数，计数为 1 的即为外边界。
    use std::collections::HashMap;
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for tri in surface.chunks_exact(3) {
        for edge in [
            (tri[0], tri[1]),
            (tri[1], tri[2]),
            (tri[2], tri[0]),
        ] {
            let key = if edge.0 < edge.1 {
                edge
            } else {
                (edge.1, edge.0)
            };
            *edges.entry(key).or_insert(0) += 1;
        }
    }
    for ((a, b), count) in edges {
        if count != 1 {
            continue;
        }
        let a = a as usize * stride;
        let b = b as usize * stride;
        // 沿剖面相邻两圈连成四边形（两个三角形），形成侧墙。
        for ring in 0..last {
            indices.extend_from_slice(&[
                (a + ring) as u32,
                (a + ring + 1) as u32,
                (b + ring + 1) as u32,
                (a + ring) as u32,
                (b + ring + 1) as u32,
                (b + ring) as u32,
            ]);
        }
    }

    // 防御性裁剪：丢弃任何超出顶点范围的索引。
    indices.retain(|&i| (i as usize) < point_count * stride);
    indices
}

/// 把 2D 采样点按剖面展开为参考顶点缓冲（与 [`write_frame_with_slots`] 的
/// 写入布局完全相同），供未动画的网格直接使用。
fn expand_positions(
    points: &[[f32; 2]],
    normals: &[[f32; 2]],
    stride: usize,
    radius: f32,
    half: f32,
) -> Vec<f32> {
    let mut out = Vec::with_capacity(points.len() * stride * 3);
    for (index, p) in points.iter().enumerate() {
        let normal = normals.get(index).copied().unwrap_or([0.0, 0.0]);
        for ring in 0..stride {
            let (offset, z) = ring_profile(ring, radius, half);
            out.push(p[0] + normal[0] * offset);
            out.push(p[1] + normal[1] * offset);
            out.push(z);
        }
    }
    out
}

/// 把拼接点缓冲拆回轮廓，按包含关系标记哪些是孔洞。
///
/// 与三角化器的分组逻辑一致：面积更大的轮廓包含代表点即视为父级。
/// 仅对带圆角的填充调用，用于决定法线偏移方向。
fn classify_holes(points: &[[f32; 2]], plans: &[SlotPlan]) -> Vec<bool> {
    let mut contours: Vec<&[[f32; 2]]> = Vec::with_capacity(plans.len());
    let mut offset = 0usize;
    for plan in plans {
        let end = (offset + plan.k).min(points.len());
        contours.push(&points[offset..end]);
        offset = end;
    }

    let mut order: Vec<usize> = (0..contours.len()).collect();
    order.sort_by(|&a, &b| {
        area(contours[b])
            .abs()
            .partial_cmp(&area(contours[a]).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut holes = vec![false; contours.len()];
    // 对每条轮廓，只看比它面积大的已处理轮廓，命中包含测试即记为孔洞。
    for (position, &index) in order.iter().enumerate() {
        if contours[index].len() < 3 || area(contours[index]).abs() < 1e-3 {
            continue;
        }
        let representative = representative_point(contours[index]);
        for &outer in &order[..position] {
            if area(contours[outer]).abs() > area(contours[index]).abs()
                && point_in_polygon(representative, contours[outer])
            {
                holes[index] = true;
                break;
            }
        }
    }
    holes
}

/// 为每个填充点计算朝盖面外的法线。
///
/// 外轮廓沿其内部方向偏移，孔洞则反向（朝实体内部），从而让圆角剖面
/// 在孔洞边缘也正确内收。`out` 会被追加 `point_count` 个二维法线。
fn fill_normals(points: &[[f32; 2]], plans: &[SlotPlan], out: &mut Vec<[f32; 2]>) {
    let mut offset = 0usize;
    for plan in plans {
        let end = (offset + plan.k).min(points.len());
        push_contour_normals(&points[offset..end], plan.hole, out);
        offset = end;
    }
}

/// 计算单条轮廓的逐点法线（相邻两段法线之和再归一化）。
///
/// 绕向由有符号面积决定，`hole` 为真时翻转方向；结果已乘好方向符号。
fn push_contour_normals(points: &[[f32; 2]], hole: bool, out: &mut Vec<[f32; 2]>) {
    let n = points.len();
    if n < 2 {
        out.extend(std::iter::repeat([0.0, 0.0]).take(n));
        return;
    }
    let direction = if area(points) >= 0.0 { 1.0 } else { -1.0 };
    let direction = if hole { -direction } else { direction };
    for i in 0..n {
        let prev = points[(i + n - 1) % n];
        let current = points[i];
        let next = points[(i + 1) % n];
        let mut nx = 0.0f32;
        let mut ny = 0.0f32;
        for (a, b) in [(prev, current), (current, next)] {
            let dx = b[0] - a[0];
            let dy = b[1] - a[1];
            let length = (dx * dx + dy * dy).sqrt();
            if length > 1e-6 {
                nx += -dy / length;
                ny += dx / length;
            }
        }
        let length = (nx * nx + ny * ny).sqrt();
        if length > 1e-6 {
            nx /= length;
            ny /= length;
        } else {
            nx = 1.0;
            ny = 0.0;
        }
        out.push([nx * direction, ny * direction]);
    }
}
