//! Scene compiler: walks Lottie layers (including precompositions), builds
//! 3D meshes and bakes every sampled frame into compact transform / colour /
//! vertex tracks that the three.js player can upload directly.

use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use serde::Deserialize;
use serde_json::Value;

use crate::lottie::{Layer, Lottie};
use crate::mesh::{self, MeshSurface, SlotPlan};
use crate::path::Contour;
use crate::prop;
use crate::shape::{self, Affine, DrawKind, Drawable, FollowContext, FollowSpecs, PathItem};

#[derive(Deserialize, Clone, Debug, Default)]
pub struct RawOptions {
    pub fps: Option<f32>,
    #[serde(default)]
    pub max_frames: Option<usize>,
    /// Extrusion half-thickness in Lottie pixels.
    pub thickness: Option<f32>,
    /// Edge smoothing radius in Lottie pixels, clamped to the half-thickness.
    #[serde(default, alias = "edgeRadius")]
    pub edge_radius: Option<f32>,
}

/// One cached topology: a signature plus the indices built for it.
pub struct MeshBucket {
    pub signature: u64,
    pub slots: Vec<SlotPlan>,
    pub indices: Vec<u32>,
    pub point_count: u32,
    pub vertex_count: u32,
}

pub struct CompiledMesh {
    pub kind: u8,
    pub static_color: [f32; 4],
    pub depth: f32,
    pub half_thickness: f32,
    pub positions: Vec<f32>,
    /// Per-frame vertices packed contiguously; `frame_offsets` holds the
    /// start of every frame (in floats) so buckets may use different counts.
    pub frame_positions: Option<Vec<f32>>,
    pub frame_offsets: Vec<u32>,
    /// Bucket index used by each frame (empty while there is only one bucket).
    pub frame_buckets: Vec<u8>,
    pub buckets: Vec<MeshBucket>,
    pub indices: Vec<u32>,
    pub color_track: Vec<f32>,
    pub transform_index: usize,
    pub render_order: u32,
    pub point_count: u32,
    pub name: String,
}

impl CompiledMesh {
    fn from_surface(
        surface: MeshSurface,
        half: f32,
        kind: u8,
        color: [f32; 4],
        depth: f32,
        transform_index: usize,
        name: String,
        animated: bool,
    ) -> CompiledMesh {
        let signature = mesh::slot_signature(&surface.slots).unwrap_or(0);
        let point_count = surface.point_count as u32;
        let vertex_count = (surface.point_count * surface.stride) as u32;
        let frames = if animated { Some(Vec::new()) } else { None };
        CompiledMesh {
            kind,
            static_color: color,
            depth,
            half_thickness: half,
            positions: surface.ref_positions,
            frame_positions: frames,
            frame_offsets: Vec::new(),
            frame_buckets: Vec::new(),
            buckets: vec![MeshBucket {
                signature,
                slots: surface.slots,
                indices: surface.indices.clone(),
                point_count,
                vertex_count,
            }],
            indices: surface.indices,
            color_track: Vec::new(),
            transform_index,
            render_order: 0,
            point_count,
            name,
        }
    }
}

pub struct Compiled {
    pub width: f32,
    pub height: f32,
    pub frame_rate: f32,
    pub ip: f32,
    pub op: f32,
    pub sample_fps: f32,
    pub frame_count: usize,
    pub name: String,
    pub conversion_us: u32,
    pub meshes: Vec<CompiledMesh>,
    pub transforms: Vec<Vec<f32>>,
}

struct Blueprint {
    mesh_index: usize,
    color: [f32; 4],
}

struct Plan<'a> {
    layer: &'a Layer,
    owner: Option<usize>,
    parent: Option<usize>,
    depth: f32,
    transform_track: usize,
    meshes: Vec<usize>,
    blueprints: Vec<Option<Blueprint>>,
    geometry_animated: bool,
    style_animated: bool,
    /// Layer opacity (own or inherited) changes across the sampled range,
    /// so mesh colours embedded the layer alpha must be baked per frame.
    opacity_animated: bool,
    ref_frame: f32,
    /// Vertex-follow null expressions on this layer's shape paths.
    follows: Option<FollowSpecs>,
}

struct Compiler<'a> {
    frame_count: usize,
    sample_start: f32,
    sample_span: f32,
    half: f32,
    edge_radius: f32,
    global: [f32; 16],
    meshes: Vec<CompiledMesh>,
    transforms: Vec<Vec<f32>>,
    plans: Vec<Plan<'a>>,
}

pub fn compile(json: &str, raw_options: RawOptions) -> Result<Compiled, String> {
    #[cfg(not(target_arch = "wasm32"))]
    let started = Instant::now();
    let doc: Lottie = serde_json::from_str(json).map_err(|error| format!("JSON 解析失败: {error}"))?;
    if doc.layers.is_empty() {
        return Err("Lottie 文件中没有图层".to_string());
    }
    let width = if doc.w > 0.0 { doc.w } else { 512.0 };
    let height = if doc.h > 0.0 { doc.h } else { 512.0 };
    let frame_rate = if doc.fr >= 1.0 { doc.fr } else { 30.0 };
    let ip = doc.ip;
    let op = if doc.op > ip { doc.op } else { ip + frame_rate * 3.0 };

    let sample_fps = raw_options
        .fps
        .unwrap_or(frame_rate)
        .clamp(6.0, 120.0);
    let raw_frames = (((op - ip) / frame_rate) * sample_fps).round() as usize;
    let max_frames = raw_options.max_frames.unwrap_or(600).clamp(1, 1200);
    let frame_count = raw_frames.max(1).min(max_frames);

    let half = raw_options
        .thickness
        .unwrap_or_else(|| width.max(height) * 0.012)
        .clamp(0.01, width.max(height) * 0.08);
    let edge_radius = raw_options
        .edge_radius
        .unwrap_or(0.0)
        .clamp(0.0, half);

    let global = global_matrix(width, height);
    let global_ref = (ip + op) * 0.5;

    let mut compiler = Compiler {
        frame_count,
        sample_start: ip,
        sample_span: op - ip,
        half,
        edge_radius,
        global,
        meshes: Vec::new(),
        transforms: Vec::new(),
        plans: Vec::new(),
    };

    let layers = &doc.layers;
    compiler.build_comp(&doc, layers, None, global_ref);
    compiler.mark_opacity_animation();
    compiler.allocate_color_tracks();

    // Performance guard: animated geometry is baked per frame. Cap the total
    // float budget so heavy scenes stay inside a few tens of MB instead of
    // hundreds, lowering the sampling rate only when necessary.
    let animated_floats: usize = compiler
        .meshes
        .iter()
        .filter(|mesh| mesh.frame_positions.is_some())
        .map(|mesh| mesh.positions.len())
        .sum();
    if animated_floats > 0 {
        const BUDGET_FLOATS: usize = 8_000_000;
        let allowed = (BUDGET_FLOATS / animated_floats).max(24);
        if allowed < compiler.frame_count {
            compiler.resize_frames(allowed);
        }
    }

    let sample_fps = compiler.frame_count as f32 / ((op - ip) / frame_rate).max(1e-4);
    let frame_count = compiler.frame_count;
    compiler.bake();

    #[cfg(not(target_arch = "wasm32"))]
    let conversion_us = started.elapsed().as_micros() as u32;
    #[cfg(target_arch = "wasm32")]
    let conversion_us = 0;

    let mut compiled = Compiled {
        width,
        height,
        frame_rate,
        ip,
        op,
        sample_fps,
        frame_count,
        name: doc.nm.clone(),
        conversion_us,
        meshes: compiler.meshes,
        transforms: compiler.transforms,
    };

    for (index, mesh) in compiled.meshes.iter_mut().enumerate() {
        mesh.render_order = (((mesh.depth + 1.0) * 10000.0) as u32).saturating_add(index as u32);
    }
    if compiled.meshes.is_empty() {
        return Err("没有可转换的矢量图形（可能只包含图片/文本图层）".to_string());
    }
    Ok(compiled)
}

impl<'a> Compiler<'a> {
    fn build_comp(
        &mut self,
        doc: &'a Lottie,
        layers: &'a [Layer],
        owner: Option<usize>,
        owner_ref: f32,
    ) {
        let count = layers.len().max(1);
        let mut ids = Vec::new();
        for (index, layer) in layers.iter().enumerate() {
            if layer.hd {
                continue;
            }
            let mut ref_frame = owner_ref;
            if layer.op > layer.ip {
                ref_frame = ref_frame.clamp(layer.ip, (layer.op - 0.001).max(layer.ip));
            }
            let local_t = if count > 1 {
                index as f32 / (count - 1) as f32
            } else {
                0.5
            };
            let depth = match owner {
                Some(parent) => self.plans[parent].depth + (0.5 - local_t) * 0.16,
                None => 0.5 - local_t,
            }
            .clamp(-0.65, 0.65);
            let track = self.transforms.len();
            self.transforms
                .push(vec![0.0; self.frame_count * 16]);
            let id = self.plans.len();
            self.plans.push(Plan {
                layer,
                owner,
                parent: None,
                depth,
                transform_track: track,
                meshes: Vec::new(),
                blueprints: Vec::new(),
                geometry_animated: false,
                style_animated: false,
                opacity_animated: false,
                ref_frame,
                follows: None,
            });
            ids.push(id);
        }

        let mut by_ind: HashMap<i64, usize> = HashMap::new();
        for &id in &ids {
            by_ind.insert(self.plans[id].layer.ind, id);
        }
        for &id in &ids {
            let parent_ind = self.plans[id].layer.parent;
            if let Some(parent_ind) = parent_ind {
                if let Some(&parent) = by_ind.get(&parent_ind) {
                    self.plans[id].parent = Some(parent);
                }
            }
        }

        for id in ids {
            self.build_content(doc, id);
        }
    }

    fn build_content(&mut self, doc: &'a Lottie, id: usize) {
        let layer = self.plans[id].layer;
        match layer.ty {
            4 => self.build_shape_layer(id, layer),
            1 => self.build_solid_layer(id, layer),
            0 => {
                let Some(asset) = doc.asset(&layer.ref_id) else {
                    return;
                };
                if asset.layers.is_empty() {
                    return;
                }
                let stretch = if layer.sr.abs() > 1e-3 { layer.sr } else { 1.0 };
                let child_ref = (self.plans[id].ref_frame - layer.st) / stretch;
                self.build_comp(doc, &asset.layers, Some(id), child_ref);
            }
            _ => {}
        }
    }

    fn build_shape_layer(&mut self, id: usize, layer: &'a Layer) {
        // Reference geometry is sampled in layer-local time, like property eval.
        let frame = self.plans[id].ref_frame - layer.st;
        let follows = self.collect_follows(id, &layer.shapes);
        let geometry_animated = shape::geometry_animated(&layer.shapes)
            || follows.as_ref().map(|specs| !specs.map.is_empty()).unwrap_or(false);
        let style_animated = shape::style_animated(&layer.shapes);
        {
            let plan = &mut self.plans[id];
            plan.geometry_animated = geometry_animated;
            plan.style_animated = style_animated;
            plan.follows = follows.clone();
        }

        // Build the reference frame with the same vertex-follow context used
        // while baking, so topology matches the real animation.
        let result = match &follows {
            Some(specs) => {
                let plan_count = self.plans.len();
                let mut chain_cache = vec![None; plan_count];
                let mut frame_cache = vec![None; plan_count];
                let mut positions = vec![None; plan_count];
                fill_follow_positions(
                    &self.plans,
                    specs,
                    self.plans[id].ref_frame,
                    &mut chain_cache,
                    &mut frame_cache,
                    &mut positions,
                );
                let inverse = layer_inverse(
                    &self.plans,
                    id,
                    self.plans[id].ref_frame,
                    &mut chain_cache,
                    &mut frame_cache,
                );
                let ctx = FollowContext {
                    specs,
                    layer_inverse: inverse,
                    positions: &positions,
                };
                shape::eval_shapes_follow(&layer.shapes, frame, &ctx)
            }
            None => shape::eval_shapes(&layer.shapes, frame),
        };

        for drawable in result.drawables {
            let Some(surface) = mesh::build_surface(&drawable, self.half, self.edge_radius) else {
                self.plans[id].blueprints.push(None);
                continue;
            };
            let color = [
                drawable.color[0],
                drawable.color[1],
                drawable.color[2],
                drawable.color[3] * drawable.opacity,
            ];
            let mesh_index = self.meshes.len();
            let name = format!("{} · {}", layer.nm, drawable.name);
            self.meshes.push(CompiledMesh::from_surface(
                surface,
                self.half,
                if drawable.kind == DrawKind::Fill { 0 } else { 1 },
                color,
                self.plans[id].depth,
                self.plans[id].transform_track,
                name,
                geometry_animated,
            ));
            self.plans[id].meshes.push(mesh_index);
            self.plans[id]
                .blueprints
                .push(Some(Blueprint { mesh_index, color }));
        }
    }

    fn build_solid_layer(&mut self, id: usize, layer: &'a Layer) {
        let sw = layer.solid_width.max(1.0);
        let sh = layer.solid_height.max(1.0);
        let rgb = parse_hex(&layer.solid_color);
        let contour = Contour {
            pts: vec![
                [0.0, 0.0],
                [sw, 0.0],
                [sw, sh],
                [0.0, sh],
            ],
            closed: true,
        };
        let drawable = Drawable {
            kind: DrawKind::Fill,
            color: [rgb[0], rgb[1], rgb[2], 1.0],
            opacity: 1.0,
            width: 0.0,
            cap: 2,
            join: 2,
            miter: 4.0,
            name: layer.nm.clone(),
            paths: vec![PathItem {
                contour,
                trim: None,
            }],
        };
        let Some(surface) = mesh::build_surface(&drawable, self.half, self.edge_radius) else {
            return;
        };
        let mesh_index = self.meshes.len();
        self.meshes.push(CompiledMesh::from_surface(
            surface,
            self.half,
            0,
            [rgb[0], rgb[1], rgb[2], 1.0],
            self.plans[id].depth,
            self.plans[id].transform_track,
            layer.nm.clone(),
            false,
        ));
        self.plans[id].meshes.push(mesh_index);
        self.plans[id].blueprints.push(Some(Blueprint {
            mesh_index,
            color: [rgb[0], rgb[1], rgb[2], 1.0],
        }));
    }

    /// Shrink the baked frame budget. Only valid before [`Compiler::bake`],
    /// while all tracks still hold placeholder zeros.
    fn resize_frames(&mut self, new_count: usize) {
        if new_count >= self.frame_count {
            return;
        }
        self.frame_count = new_count;
        for track in self.transforms.iter_mut() {
            track.resize(new_count * 16, 0.0);
        }
        for mesh in self.meshes.iter_mut() {
            // Frame vertices are appended during bake (sizes may vary per
            // topology bucket), so only pre-allocated tracks are resized.
            if mesh.frame_positions.is_some() {
                mesh.frame_positions = Some(Vec::new());
                mesh.frame_offsets.clear();
                mesh.frame_buckets.clear();
            }
            if !mesh.color_track.is_empty() {
                mesh.color_track.resize(new_count * 4, 0.0);
            }
        }
    }

    /// Walk the shape tree and resolve `vertex follows null` expressions to
    /// layer plan ids of the same composition.
    fn collect_follows(&self, id: usize, shapes: &[Value]) -> Option<FollowSpecs> {
        let owner = self.plans[id].owner;
        let mut specs = FollowSpecs::default();
        let mut path = Vec::new();
        collect_follows_inner(&self.plans, owner, shapes, &mut path, &mut specs);
        if specs.map.is_empty() {
            None
        } else {
            Some(specs)
        }
    }

    /// Mark every plan whose layer alpha varies over the sampled range. Owners
    /// always own a lower plan index than their composition children, so one
    /// forward pass is enough to propagate inherited opacity animation.
    fn mark_opacity_animation(&mut self) {
        let sample_start = self.sample_start;
        let sample_end = self.sample_start + self.sample_span;
        for id in 0..self.plans.len() {
            let owner_animated = self.plans[id]
                .owner
                .map(|owner| self.plans[owner].opacity_animated)
                .unwrap_or(false);
            let shifted = self.plans[id]
                .owner
                .map(|owner| {
                    let owner_layer = self.plans[owner].layer;
                    owner_layer.st.abs() > 1e-3 || (owner_layer.sr - 1.0).abs() > 1e-3
                })
                .unwrap_or(false);
            let (own, full_visible) = {
                let layer = self.plans[id].layer;
                (
                    layer.ks.get("o").map(shape::is_animated).unwrap_or(false),
                    layer.op <= layer.ip
                        || (layer.ip <= sample_start && layer.op >= sample_end),
                )
            };
            self.plans[id].opacity_animated =
                own || owner_animated || shifted || !full_visible;
        }
    }

    /// Allocate per-frame colour tracks only for meshes whose colour or layer
    /// alpha actually changes; constant colours stay in the mesh record.
    fn allocate_color_tracks(&mut self) {
        let mut flags: Vec<(usize, bool)> = Vec::new();
        for plan in &self.plans {
            let animated = plan.style_animated || plan.opacity_animated;
            for blueprint in plan.blueprints.iter().flatten() {
                flags.push((blueprint.mesh_index, animated));
            }
        }
        for (index, animated) in flags {
            let track_len = if animated { self.frame_count * 4 } else { 0 };
            self.meshes[index].color_track = vec![0.0; track_len];
        }
    }

    fn bake(&mut self) {
        let frame_count = self.frame_count;
        let sample_start = self.sample_start;
        let sample_span = self.sample_span;
        let plan_count = self.plans.len();
        let mesh_plans: Vec<usize> = (0..plan_count)
            .filter(|&id| !self.plans[id].blueprints.is_empty())
            .collect();

        // Caches are reused for every frame instead of reallocated per frame.
        let mut world_cache: Vec<Option<[f32; 16]>> = vec![None; plan_count];
        let mut chain_cache: Vec<Option<[f32; 16]>> = vec![None; plan_count];
        let mut opacity_cache: Vec<Option<f32>> = vec![None; plan_count];
        let mut frame_cache: Vec<Option<f32>> = vec![None; plan_count];
        let mut follow_positions: Vec<Option<[f32; 2]>> = vec![None; plan_count];
        let mut scratch = mesh::FrameScratch::default();

        for frame_index in 0..frame_count {
            let root_frame = sample_start + sample_span * (frame_index as f32 / frame_count as f32);
            for cache in world_cache.iter_mut() {
                *cache = None;
            }
            for cache in chain_cache.iter_mut() {
                *cache = None;
            }
            for cache in opacity_cache.iter_mut() {
                *cache = None;
            }
            for cache in frame_cache.iter_mut() {
                *cache = None;
            }

            for id in 0..plan_count {
                let (matrix, _opacity) = plan_world(
                    &self.plans,
                    &self.global,
                    id,
                    root_frame,
                    &mut world_cache,
                    &mut chain_cache,
                    &mut opacity_cache,
                    &mut frame_cache,
                );
                let track = self.plans[id].transform_track;
                let slot = &mut self.transforms[track]
                    [frame_index * 16..(frame_index + 1) * 16];
                slot.copy_from_slice(&matrix);
            }

            let plans = &self.plans;
            let meshes = &mut self.meshes;
            let edge_radius = self.edge_radius;
            for &id in &mesh_plans {
                let plan = &plans[id];
                let opacity = opacity_cache[id].unwrap_or(0.0);
                let local_frame = frame_cache[id].unwrap_or(root_frame);
                let geometry_animated = plan.geometry_animated;
                // Geometry-only / style-only animations avoid evaluating what
                // they do not need: style animation skips path flattening.
                let evaluated = if geometry_animated {
                    match &plan.follows {
                        Some(specs) => {
                            fill_follow_positions(
                                plans,
                                specs,
                                root_frame,
                                &mut chain_cache,
                                &mut frame_cache,
                                &mut follow_positions,
                            );
                            let inverse = layer_inverse(
                                plans,
                                id,
                                root_frame,
                                &mut chain_cache,
                                &mut frame_cache,
                            );
                            let ctx = FollowContext {
                                specs,
                                layer_inverse: inverse,
                                positions: &follow_positions,
                            };
                            Some(shape::eval_shapes_follow(
                                &plan.layer.shapes,
                                local_frame,
                                &ctx,
                            ))
                        }
                        None => Some(shape::eval_shapes(&plan.layer.shapes, local_frame)),
                    }
                } else if plan.style_animated {
                    Some(shape::eval_styles(&plan.layer.shapes, local_frame))
                } else {
                    None
                };
                let dynamic_color = plan.style_animated || plan.opacity_animated;

                for (drawable_index, blueprint) in plan.blueprints.iter().enumerate() {
                    let Some(blueprint) = blueprint else {
                        continue;
                    };
                    let mesh = &mut meshes[blueprint.mesh_index];

                    if dynamic_color {
                        let drawable = evaluated
                            .as_ref()
                            .and_then(|result| result.drawables.get(drawable_index));
                        let color = match drawable {
                            Some(drawable) => [
                                drawable.color[0],
                                drawable.color[1],
                                drawable.color[2],
                                drawable.color[3] * drawable.opacity * opacity,
                            ],
                            None => [
                                blueprint.color[0],
                                blueprint.color[1],
                                blueprint.color[2],
                                blueprint.color[3] * opacity,
                            ],
                        };
                        let base = frame_index * 4;
                        mesh.color_track[base..base + 4].copy_from_slice(&color);
                    } else if frame_index == 0 {
                        // Constant colour: fold the static layer alpha into the
                        // mesh record so no colour track is needed at all.
                        mesh.static_color = [
                            blueprint.color[0],
                            blueprint.color[1],
                            blueprint.color[2],
                            blueprint.color[3] * opacity,
                        ];
                    }

                    if geometry_animated {
                        let start = mesh
                            .frame_positions
                            .as_ref()
                            .map(|track| track.len())
                            .unwrap_or(0);
                        let mut bucket_index = 0usize;
                        if let Some(drawable) = evaluated
                            .as_ref()
                            .and_then(|result| result.drawables.get(drawable_index))
                        {
                            let half = mesh.half_thickness;
                            let slots = shape::drawable_slots(drawable);
                            if !slots.is_empty() {
                                let natural = mesh::plan_slots(&slots);
                                let signature = mesh::slot_signature(&natural);
                                if let Some(sig) = signature {
                                    if let Some(position) = mesh
                                        .buckets
                                        .iter()
                                        .position(|bucket| bucket.signature == sig)
                                    {
                                        bucket_index = position;
                                    } else if mesh.buckets.len() < mesh::MAX_BUCKETS {
                                        if let Some(surface) = mesh::build_surface_from_slots(
                                            drawable,
                                            &slots,
                                            natural,
                                            half,
                                            edge_radius,
                                        ) {
                                            mesh.point_count = mesh
                                                .point_count
                                                .max(surface.point_count as u32);
                                            mesh.buckets.push(MeshBucket {
                                                signature: sig,
                                                slots: surface.slots,
                                                indices: surface.indices,
                                                point_count: surface.point_count as u32,
                                                vertex_count: (surface.point_count * surface.stride)
                                                    as u32,
                                            });
                                            bucket_index = mesh.buckets.len() - 1;
                                        }
                                    }
                                }

                                let (count, bucket_plans) = {
                                    let bucket = &mesh.buckets[bucket_index];
                                    (bucket.vertex_count as usize * 3, bucket.slots.as_slice())
                                };
                                if let Some(track) = mesh.frame_positions.as_mut() {
                                    let write_start = track.len();
                                    track.resize(write_start + count, 0.0);
                                    if !mesh::write_frame_with_slots(
                                        drawable,
                                        &slots,
                                        bucket_plans,
                                        half,
                                        edge_radius,
                                        &mut track[write_start..],
                                        &mut scratch,
                                    ) {
                                        track.truncate(write_start);
                                    }
                                }
                            }
                        }
                        // Offsets stay aligned even when a frame has no drawable.
                        mesh.frame_offsets.push(start as u32);
                        mesh.frame_buckets.push(bucket_index as u8);
                    }
                }
            }
        }

        // Close the per-frame offset table and pad the initial positions so
        // every bucket (which may need more vertices) fits the vertex buffer.
        for mesh in self.meshes.iter_mut() {
            if mesh.frame_positions.is_none() {
                mesh.frame_offsets.clear();
                mesh.frame_buckets.clear();
                continue;
            }
            let max_vertex = mesh
                .buckets
                .iter()
                .map(|bucket| bucket.vertex_count)
                .max()
                .unwrap_or(0);
            mesh.point_count = mesh
                .point_count
                .max(mesh.buckets.iter().map(|bucket| bucket.point_count).max().unwrap_or(0));
            let needed = max_vertex as usize * 3;
            if mesh.positions.len() < needed {
                mesh.positions.resize(needed, 0.0);
            }
            let total = mesh.frame_positions.as_ref().map_or(0, |track| track.len());
            mesh.frame_offsets.push(total as u32);
        }
    }
}

/* ------------------------------------------------------------------ */
/* Vertex-follow expressions                                           */
/* ------------------------------------------------------------------ */

/// Null layer names are resolved within the shape layer's own composition.
fn collect_follows_inner(
    plans: &[Plan],
    owner: Option<usize>,
    items: &[Value],
    path: &mut Vec<usize>,
    specs: &mut FollowSpecs,
) {
    const MISSING: usize = usize::MAX;
    for (index, item) in items.iter().enumerate() {
        match item.get("ty").and_then(Value::as_str).unwrap_or("") {
            "sh" => {
                if let Some(names) = shape::vertex_follow_names(item) {
                    path.push(index);
                    let mut targets = Vec::with_capacity(names.len());
                    for name in names {
                        targets.push(
                            plans
                                .iter()
                                .position(|plan| {
                                    plan.owner == owner && plan.layer.nm == name
                                })
                                .unwrap_or(MISSING),
                        );
                    }
                    specs.map.insert(path.clone(), targets);
                    path.pop();
                }
            }
            "gr" => {
                if let Some(children) = item.get("it").and_then(Value::as_array) {
                    path.push(index);
                    collect_follows_inner(plans, owner, children, path, specs);
                    path.pop();
                }
            }
            _ => {}
        }
    }
}

/// Composition-space anchor position of a layer with its parent chain baked.
fn comp_anchor(
    plans: &[Plan],
    id: usize,
    root_frame: f32,
    chain_cache: &mut [Option<[f32; 16]>],
    frame_cache: &mut [Option<f32>],
) -> [f32; 2] {
    let matrix = plan_chain(plans, id, root_frame, chain_cache, frame_cache);
    let frame = plan_local_frame(plans, id, root_frame);
    let anchor = prop::eval_in(&plans[id].layer.ks, "a", frame, &[0.0, 0.0, 0.0]);
    [
        matrix[0] * anchor[0] + matrix[4] * anchor[1] + matrix[12],
        matrix[1] * anchor[0] + matrix[5] * anchor[1] + matrix[13],
    ]
}

/// Resolve every referenced null layer once per frame into `out`.
fn fill_follow_positions(
    plans: &[Plan],
    specs: &FollowSpecs,
    root_frame: f32,
    chain_cache: &mut [Option<[f32; 16]>],
    frame_cache: &mut [Option<f32>],
    out: &mut Vec<Option<[f32; 2]>>,
) {
    out.clear();
    out.resize(plans.len(), None);
    for targets in specs.map.values() {
        for &target in targets {
            if target < out.len() && out[target].is_none() {
                out[target] = Some(comp_anchor(
                    plans,
                    target,
                    root_frame,
                    chain_cache,
                    frame_cache,
                ));
            }
        }
    }
}

/// Inverse of the shape layer's composition matrix (`fromCompToSurface`).
fn layer_inverse(
    plans: &[Plan],
    id: usize,
    root_frame: f32,
    chain_cache: &mut [Option<[f32; 16]>],
    frame_cache: &mut [Option<f32>],
) -> Affine {
    let matrix = plan_chain(plans, id, root_frame, chain_cache, frame_cache);
    Affine {
        a: matrix[0],
        b: matrix[1],
        c: matrix[4],
        d: matrix[5],
        tx: matrix[12],
        ty: matrix[13],
    }
    .inverse()
    .unwrap_or(Affine::IDENTITY)
}

/* ------------------------------------------------------------------ */
/* Matrices                                                            */
/* ------------------------------------------------------------------ */

fn global_matrix(width: f32, height: f32) -> [f32; 16] {
    let size = width.max(height).max(1.0);
    let scale = 3.6 / size;
    [
        scale, 0.0, 0.0, 0.0,
        0.0, -scale, 0.0, 0.0,
        0.0, 0.0, scale, 0.0,
        -scale * width * 0.5, scale * height * 0.5, 0.0, 1.0,
    ]
}

fn mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            let mut sum = 0.0;
            for k in 0..4 {
                sum += a[k * 4 + row] * b[col * 4 + k];
            }
            out[col * 4 + row] = sum;
        }
    }
    out
}

fn affine_matrix(matrix: Affine) -> [f32; 16] {
    [
        matrix.a, matrix.b, 0.0, 0.0,
        matrix.c, matrix.d, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        matrix.tx, matrix.ty, 0.0, 1.0,
    ]
}

fn plan_local_frame(plans: &[Plan], id: usize, root_frame: f32) -> f32 {
    let parent_frame = match plans[id].owner {
        Some(owner) => {
            // The owning precomposition renders its children in time divided
            // by its stretch (`renderedFrame = num / sr`).
            let owner_frame = plan_local_frame(plans, owner, root_frame);
            let owner_layer = plans[owner].layer;
            let stretch = if owner_layer.sr.abs() > 1e-3 {
                owner_layer.sr
            } else {
                1.0
            };
            owner_frame / stretch
        }
        None => root_frame,
    };
    // lottie-web evaluates a layer's properties relative to its start time.
    parent_frame - plans[id].layer.st
}

fn visible(layer: &Layer, frame: f32) -> bool {
    if layer.op <= layer.ip {
        true
    } else {
        frame >= layer.ip && frame < layer.op
    }
}

fn layer_opacity(layer: &Layer, frame: f32) -> f32 {
    prop::eval_scalar(&layer.ks, "o", frame, 100.0).clamp(0.0, 100.0) / 100.0
}

fn plan_chain(
    plans: &[Plan],
    id: usize,
    root_frame: f32,
    cache: &mut [Option<[f32; 16]>],
    frame_cache: &mut [Option<f32>],
) -> [f32; 16] {
    if let Some(matrix) = cache[id] {
        return matrix;
    }
    let frame = plan_local_frame(plans, id, root_frame);
    let local = affine_matrix(shape::transform_matrix(&plans[id].layer.ks, frame));
    let matrix = match plans[id].parent {
        Some(parent) => {
            let parent_matrix = plan_chain(plans, parent, root_frame, cache, frame_cache);
            mat_mul(&parent_matrix, &local)
        }
        None => local,
    };
    cache[id] = Some(matrix);
    frame_cache[id] = Some(frame);
    matrix
}

fn plan_world(
    plans: &[Plan],
    global: &[f32; 16],
    id: usize,
    root_frame: f32,
    world_cache: &mut Vec<Option<[f32; 16]>>,
    chain_cache: &mut Vec<Option<[f32; 16]>>,
    opacity_cache: &mut Vec<Option<f32>>,
    frame_cache: &mut Vec<Option<f32>>,
) -> ([f32; 16], f32) {
    if let (Some(matrix), Some(opacity)) = (world_cache[id], opacity_cache[id]) {
        return (matrix, opacity);
    }
    let chain = plan_chain(plans, id, root_frame, chain_cache, frame_cache);
    let (base, base_opacity) = match plans[id].owner {
        Some(owner) => plan_world(
            plans,
            global,
            owner,
            root_frame,
            world_cache,
            chain_cache,
            opacity_cache,
            frame_cache,
        ),
        None => (*global, 1.0),
    };
    let mut world = mat_mul(&base, &chain);
    // Bake the layer depth into the track so the player can upload the matrix
    // as-is (the depth slider only adds a delta).
    world[14] += plans[id].depth;
    let frame = frame_cache[id].unwrap_or_else(|| plan_local_frame(plans, id, root_frame));
    // `ip` / `op` live in the parent composition's time, not layer-local time.
    let comp_frame = frame + plans[id].layer.st;
    let own = if visible(plans[id].layer, comp_frame) {
        layer_opacity(plans[id].layer, frame)
    } else {
        0.0
    };
    let opacity = base_opacity * own;
    world_cache[id] = Some(world);
    opacity_cache[id] = Some(opacity);
    (world, opacity)
}

fn parse_hex(value: &str) -> [f32; 3] {
    let text = value.trim().trim_start_matches('#');
    if text.len() >= 6 {
        if let Ok(number) = u32::from_str_radix(&text[0..6], 16) {
            return [
                ((number >> 16) & 0xff) as f32 / 255.0,
                ((number >> 8) & 0xff) as f32 / 255.0,
                (number & 0xff) as f32 / 255.0,
            ];
        }
    }
    [0.8, 0.8, 0.8]
}

/// Layer types that this converter understands.
pub fn is_supported_layer(layer: &Value) -> bool {
    matches!(
        layer.get("ty").and_then(Value::as_i64).unwrap_or(-1),
        0 | 1 | 4
    )
}
