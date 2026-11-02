//! Mesh construction: triangulate fills, build stroke ribbons, and extrude
//! both into a closed volume with front/back caps and boundary walls.

use std::borrow::Cow;

use crate::path::{self, Contour};
use crate::shape::{DrawKind, Drawable};

/// Number of segments in each quarter-round of a smoothed edge.
const ROUND_SEGMENTS: usize = 3;

#[derive(Clone, Debug)]
pub struct SlotPlan {
    pub k: usize,
    pub closed: bool,
    /// Hole contour of a compound fill; its offset direction is flipped.
    pub hole: bool,
}

pub struct MeshSurface {
    pub slots: Vec<SlotPlan>,
    pub point_count: usize,
    /// Vertices per contour point along the extruded edge profile.
    pub stride: usize,
    pub indices: Vec<u32>,
    pub ref_positions: Vec<f32>,
}

/// Vertices generated per point: 2 for a sharp edge, more when rounded.
fn profile_stride(radius: f32) -> usize {
    if radius > 1e-4 {
        2 * ROUND_SEGMENTS + 2
    } else {
        2
    }
}

/// Offset along the inward normal and Z for one profile ring.
fn ring_profile(index: usize, radius: f32, half: f32) -> (f32, f32) {
    if radius <= 1e-4 {
        return (0.0, if index == 0 { half } else { -half });
    }
    if index <= ROUND_SEGMENTS {
        // Upper quarter-round: side wall -> top cap boundary.
        let theta = (1.0 - index as f32 / ROUND_SEGMENTS as f32)
            * std::f32::consts::FRAC_PI_2;
        (
            radius * (1.0 - theta.cos()),
            (half - radius) + radius * theta.sin(),
        )
    } else {
        // Lower quarter-round: bottom cap boundary -> side wall.
        let local = index - (ROUND_SEGMENTS + 1);
        let theta = local as f32 / ROUND_SEGMENTS as f32 * std::f32::consts::FRAC_PI_2;
        (
            radius * (1.0 - theta.cos()),
            -(half - radius) - radius * theta.sin(),
        )
    }
}

/// Build the reference surface and its fixed topology.
pub fn build_surface(
    drawable: &Drawable,
    half_thickness: f32,
    edge_radius: f32,
) -> Option<MeshSurface> {
    let slots = crate::shape::drawable_slots(drawable);
    if slots.is_empty() {
        return None;
    }

    // Only fills get rounded edges; stroke ribbons keep the sharp profile.
    let radius = if drawable.kind == DrawKind::Fill {
        edge_radius.clamp(0.0, half_thickness)
    } else {
        0.0
    };
    let stride = profile_stride(radius);

    let mut plans: Vec<SlotPlan> = slots
        .iter()
        .map(|(contour, closed, _)| {
            let length = contour.length();
            let k = if length < 1e-4 {
                4
            } else {
                ((length / 4.0).ceil() as usize).clamp(6, 128)
            };
            SlotPlan {
                k,
                closed: *closed,
                hole: false,
            }
        })
        .collect();

    let points = build_points(drawable, &slots, &plans)?;
    if drawable.kind == DrawKind::Fill && radius > 0.0 {
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
    let indices = extrude(&surface_indices, point_count, stride);
    let mut normals = Vec::new();
    if radius > 0.0 {
        normals.reserve(point_count);
        fill_normals(&points, &plans, &mut normals);
    }
    let ref_positions = expand_positions(&points, &normals, stride, radius, half_thickness);

    Some(MeshSurface {
        slots: plans,
        point_count,
        stride,
        indices,
        ref_positions,
    })
}

/// Reusable buffers for per-frame geometry baking. One instance is shared by
/// all meshes, so a full bake does not allocate per frame anymore.
#[derive(Default)]
pub struct FrameScratch {
    sampled: Vec<Vec<[f32; 2]>>,
    points: Vec<[f32; 2]>,
    normals: Vec<[f32; 2]>,
}

/// Rebuild vertex positions for another frame, writing straight into the
/// destination track slice while keeping the reference topology.
pub fn write_frame(
    drawable: &Drawable,
    plans: &[SlotPlan],
    half_thickness: f32,
    edge_radius: f32,
    out: &mut [f32],
    scratch: &mut FrameScratch,
) -> bool {
    let slots = crate::shape::drawable_slots(drawable);
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
    if points.len() * stride * 3 > out.len() {
        return false;
    }
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

fn build_points(
    drawable: &Drawable,
    slots: &[(Cow<'_, Contour>, bool, bool)],
    plans: &[SlotPlan],
) -> Option<Vec<[f32; 2]>> {
    let mut sampled: Vec<Vec<[f32; 2]>> = Vec::with_capacity(slots.len());
    for (index, (contour, closed, _)) in slots.iter().enumerate() {
        let k = plans[index].k;
        // For fills, the polygon is closed by the tessellator, but sampling
        // stays open for trimmed paths so the arc length is correct.
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

fn concat_points(chunks: &[Vec<[f32; 2]>]) -> Vec<[f32; 2]> {
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend_from_slice(chunk);
    }
    out
}

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
            // Miter factor from the angle between adjacent segments.
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
            let miter = (half / dot).min(half * drawable.miter.max(1.0));
            out.push([points[i][0] + nx * miter, points[i][1] + ny * miter]);
            out.push([points[i][0] - nx * miter, points[i][1] - ny * miter]);
        }
    }
    !out.is_empty()
}

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

fn tessellate_fill(points: &[[f32; 2]], plans: &[SlotPlan]) -> Vec<u32> {
    // Split the point buffer back into contours and group holes by containment.
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
    let mut indices = Vec::new();
    for group in groups {
        let outer = group[0];
        if outer_of[outer] != Some(outer) {
            continue;
        }
        let mut data: Vec<f64> = Vec::new();
        let mut local_to_global: Vec<u32> = Vec::new();
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
    indices
}

fn contour_offsets(plans: &[SlotPlan]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(plans.len());
    let mut offset = 0usize;
    for plan in plans {
        offsets.push(offset);
        offset += plan.k;
    }
    offsets
}

fn area(contour: &[[f32; 2]]) -> f32 {
    let mut sum = 0.0;
    for i in 0..contour.len() {
        let a = contour[i];
        let b = contour[(i + 1) % contour.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum * 0.5
}

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

/// Extrude a flat triangle surface: front cap, back cap, boundary walls.
fn extrude(surface: &[u32], point_count: usize, stride: usize) -> Vec<u32> {
    let mut indices = Vec::with_capacity(surface.len() * 2 + 64);
    let last = stride - 1;

    // Front and back caps.
    for tri in surface.chunks_exact(3) {
        indices.extend_from_slice(&[
            (tri[0] as usize * stride) as u32,
            (tri[1] as usize * stride) as u32,
            (tri[2] as usize * stride) as u32,
        ]);
    }
    for tri in surface.chunks_exact(3) {
        indices.extend_from_slice(&[
            (tri[2] as usize * stride + last) as u32,
            (tri[1] as usize * stride + last) as u32,
            (tri[0] as usize * stride + last) as u32,
        ]);
    }

    // Boundary edges: undirected edges referenced exactly once.
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

    indices.retain(|&i| (i as usize) < point_count * stride);
    indices
}

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

/// Split the concatenated fill buffer back into contours and mark which ones
/// are holes, mirroring the containment grouping used by the tessellator.
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

/// Outward-from-cap normals for every fill point. Outer contours offset
/// toward their interior, holes offset away from theirs (into the solid).
fn fill_normals(points: &[[f32; 2]], plans: &[SlotPlan], out: &mut Vec<[f32; 2]>) {
    let mut offset = 0usize;
    for plan in plans {
        let end = (offset + plan.k).min(points.len());
        push_contour_normals(&points[offset..end], plan.hole, out);
        offset = end;
    }
}

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
