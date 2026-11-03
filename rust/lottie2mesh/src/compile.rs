//! 场景编译器：遍历 Lottie 图层（含预合成），构建 3D 网格，并把每个采样帧
//! 烘焙成紧凑的变换 / 颜色 / 顶点轨迹，供 three.js 播放器直接上传。
//!
//! 模块职责：
//! * 建立图层 `Plan` 树（含预合成 owner、parent 链、深度）；
//! * 生成参考帧网格（`CompiledMesh::from_surface`）；
//! * 逐帧计算世界矩阵、透明度、几何/颜色，写入轨迹；
//! * B 方案拓扑分桶：按姿态锚点与折叠检测缓存多套 indices，帧间切换索引；
//! * 解析并烘焙 AE「顶点跟随空对象」表达式。
//!
//! 关键数据流：
//! ```text
//! JSON → Lottie → Plan 树 → 参考帧网格 → bake 逐帧轨迹
//!      → Compiled { meshes, transforms } → output::encode
//! ```
//!
//! 单位与约定：
//! * 几何 / 厚度为 Lottie 像素；时间为帧号；
//! * 矩阵为**列主序 4×4**（three.js 约定），以列向量右乘，组合顺序为父在左；
//! * 全局矩阵把画布缩放到 `3.6 / max(w, h)` 并翻转 Y；
//! * 图层深度已烘焙进矩阵的 `[14]`（Z 平移）位置，播放器只需叠加滑杆增量；

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

/// 前盖折叠判定阈值：`Σ|三角形面积| / |Σ有符号面积|` 超过该值时，
/// 认为现有三角化已不适合当前姿态，需要按当帧几何重建一套桶。
const FOLD_RATIO_THRESHOLD: f32 = 1.015;

/// 转换选项，可由 CLI 参数、C ABI 的 JSON 或默认值构造。
#[derive(Deserialize, Clone, Debug, Default)]
pub struct RawOptions {
    /// 采样帧率（帧/秒）；缺省用文档 `fr`，并钳制到 6..120。
    pub fps: Option<f32>,
    /// 烘焙帧数上限；缺省 600，钳制到 1..1200。
    #[serde(default)]
    pub max_frames: Option<usize>,
    /// 挤出半厚度，单位 Lottie 像素；缺省为 `max(w,h) * 0.012`。
    pub thickness: Option<f32>,
    /// 边缘圆角半径，单位 Lottie 像素，钳制到半厚度以内；缺省 0。
    #[serde(default, alias = "edgeRadius")]
    pub edge_radius: Option<f32>,
    /// 轮廓采样间距，单位 Lottie 像素/点；越小曲线越平滑，缺省 3.0，
    /// 钳制到 1..8。`curveSpacing` 为 camelCase 别名。
    #[serde(default, alias = "curveSpacing")]
    pub curve_spacing: Option<f32>,
}

/// 一套缓存拓扑：该姿态下的采样计划、indices 与顶点规模。
///
/// 同一个网格最多 [`mesh::MAX_BUCKETS`] 套；各帧通过 `frame_buckets`
/// 指向自己使用的那一套，运行时切换索引即可。
pub struct MeshBucket {
    pub slots: Vec<SlotPlan>,
    pub indices: Vec<u32>,
    pub point_count: u32,
    pub vertex_count: u32,
}

/// 编译后的单个网格。
pub struct CompiledMesh {
    /// 绘制类型：0 填充、1 描边。
    pub kind: u8,
    /// 静态颜色 RGBA（各 0..1）；动画颜色走 `color_track`。
    pub static_color: [f32; 4],
    /// 图层深度，用于排序与 Z 偏移。
    pub depth: f32,
    /// 挤出半厚度（Lottie 像素）。
    pub half_thickness: f32,
    /// 参考顶点缓冲 `[x, y, z]`，长度按最大桶的顶点数对齐。
    pub positions: Vec<f32>,
    /// 逐帧顶点连续打包；`frame_offsets` 记录每帧起点（单位：float），
    /// 因此不同拓扑桶可以有不同顶点数。
    pub frame_positions: Option<Vec<f32>>,
    /// 每帧在 `frame_positions` 中的起始 float 下标，末尾额外多一项表示总长度。
    pub frame_offsets: Vec<u32>,
    /// 每帧使用的桶下标（只有一个桶时为空）。
    pub frame_buckets: Vec<u8>,
    pub buckets: Vec<MeshBucket>,
    /// 基础桶的 indices 副本（兼容只上传一套索引的路径）。
    pub indices: Vec<u32>,
    /// 逐帧颜色 RGBA；为空表示使用 `static_color`。
    pub color_track: Vec<f32>,
    pub transform_index: usize,
    pub render_order: u32,
    /// 所有桶中最大的轮廓点数。
    pub point_count: u32,
    pub name: String,
}

impl CompiledMesh {
    /// 用一套新采样计划替换指定桶（基础桶按首帧姿态重建时使用）。
    fn replace_bucket(&mut self, index: usize, surface: MeshSurface) {
        let point_count = surface.point_count as u32;
        let vertex_count = (surface.point_count * surface.stride) as u32;
        let MeshSurface { slots, indices, .. } = surface;
        if index == 0 {
            // 单桶编码路径仍使用 indices 副本，需要同步。
            self.indices = indices.clone();
        }
        self.point_count = self.point_count.max(point_count);
        let bucket = &mut self.buckets[index];
        bucket.slots = slots;
        bucket.indices = indices;
        bucket.point_count = point_count;
        bucket.vertex_count = vertex_count;
    }

    /// 追加一套新桶（姿态锚点或段内拓扑变化时），返回其下标。
    fn push_bucket(&mut self, surface: MeshSurface) -> usize {
        let point_count = surface.point_count as u32;
        let vertex_count = (surface.point_count * surface.stride) as u32;
        let MeshSurface { slots, indices, .. } = surface;
        self.point_count = self.point_count.max(point_count);
        self.buckets.push(MeshBucket {
            slots,
            indices,
            point_count,
            vertex_count,
        });
        self.buckets.len() - 1
    }

    /// 由参考帧表面创建网格；`animated` 为真时预置空的逐帧顶点轨迹。
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

/// 整个场景的编译结果，编码前的最小模型。
pub struct Compiled {
    /// 画布宽高（Lottie 像素）。
    pub width: f32,
    pub height: f32,
    /// 文档帧率（帧/秒）。
    pub frame_rate: f32,
    /// 入点 / 出点（帧）。
    pub ip: f32,
    pub op: f32,
    /// 实际采样帧率（可能因预算降采样而低于请求值）。
    pub sample_fps: f32,
    pub frame_count: usize,
    pub name: String,
    /// 转换耗时（微秒）；wasm 下恒为 0。
    pub conversion_us: u32,
    pub meshes: Vec<CompiledMesh>,
    /// 每条轨迹长度 `frame_count * 16`，列主序 4×4。
    pub transforms: Vec<Vec<f32>>,
}

/// 绘制项与网格的对应关系，用于逐帧把新颜色写回正确的网格。
struct Blueprint {
    mesh_index: usize,
    color: [f32; 4],
}

/// 一个图层的编译计划（引用原始 `Layer`，避免复制 JSON）。
struct Plan<'a> {
    layer: &'a Layer,
    /// 所属预合成图层在 `plans` 中的下标；顶层为 `None`。
    owner: Option<usize>,
    /// 父图层（`parent` 指向的 `ind`）在 `plans` 中的下标。
    parent: Option<usize>,
    /// 深度（-0.65..0.65），已烘焙进世界矩阵的 Z 分量。
    depth: f32,
    /// 该图层世界矩阵轨迹在 `transforms` 中的下标。
    transform_track: usize,
    /// 该图层产生的网格下标。
    meshes: Vec<usize>,
    /// 与 drawable 一一对应的网格蓝图；构建失败的 drawable 为 `None`。
    blueprints: Vec<Option<Blueprint>>,
    geometry_animated: bool,
    style_animated: bool,
    /// 图层透明度（自身或继承）在采样区间内变化，需要逐帧烘焙颜色 alpha。
    opacity_animated: bool,
    /// 参考帧（用于构建参考几何）。
    ref_frame: f32,
    /// 该图层形状路径上的顶点跟随空对象表达式。
    follows: Option<FollowSpecs>,
}

/// 编译期的全局状态。
struct Compiler<'a> {
    frame_count: usize,
    /// 采样区间起点（帧）与跨度（帧）。
    sample_start: f32,
    sample_span: f32,
    half: f32,
    edge_radius: f32,
    /// 轮廓采样间距（Lottie 像素/点），越小采样点越多、曲线越平滑。
    curve_spacing: f32,
    /// 全局矩阵（列主序 4×4，含 3.6 缩放与 Y 翻转）。
    global: [f32; 16],
    meshes: Vec<CompiledMesh>,
    transforms: Vec<Vec<f32>>,
    plans: Vec<Plan<'a>>,
}

/// 编译入口：解析 JSON、建立图层计划、构建参考网格并逐帧烘焙。
///
/// 处理流程：
/// 1. 读取画布/时间信息，确定采样帧数与厚度；
/// 2. 递归展开图层与预合成（`build_comp`）；
/// 3. 检测透明度动画、分配颜色轨迹；
/// 4. 按 8M float 预算必要时降采样，再执行 `bake`；
/// 5. 计算渲染顺序，返回 `Compiled`。
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
    // 采样帧数 = 时长（秒）× 采样 fps；再受 max_frames（默认 600）限制。
    let raw_frames = (((op - ip) / frame_rate) * sample_fps).round() as usize;
    let max_frames = raw_options.max_frames.unwrap_or(600).clamp(1, 1200);
    let frame_count = raw_frames.max(1).min(max_frames);

    // 半厚度缺省为画布长边的 1.2%，并限制在 0.8% 以内；单位像素。
    let half = raw_options
        .thickness
        .unwrap_or_else(|| width.max(height) * 0.012)
        .clamp(0.01, width.max(height) * 0.08);
    // 圆角半径不能超过半厚度，否则剖面会自交。
    let edge_radius = raw_options
        .edge_radius
        .unwrap_or(0.0)
        .clamp(0.0, half);
    // 曲线采样间距：缺省 3 像素/点；小于 1 会迅速放大顶点/显存，大于 8 折线明显。
    let curve_spacing = raw_options
        .curve_spacing
        .unwrap_or(3.0)
        .clamp(1.0, 8.0);

    let global = global_matrix(width, height);
    // 参考帧取动画中点，兼顾开头与结尾的几何形态。
    let global_ref = (ip + op) * 0.5;

    let mut compiler = Compiler {
        frame_count,
        sample_start: ip,
        sample_span: op - ip,
        half,
        edge_radius,
        curve_spacing,
        global,
        meshes: Vec::new(),
        transforms: Vec::new(),
        plans: Vec::new(),
    };

    let layers = &doc.layers;
    compiler.build_comp(&doc, layers, None, global_ref);
    compiler.mark_opacity_animation();
    compiler.allocate_color_tracks();

    // 性能护栏：动画几何按帧烘焙。用 8M float 的总预算限制顶点数据量，
    // 重场景自动降低采样帧数（最低保留 24 帧），而不是膨胀到数百 MB。
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

    // 以实际帧数反推有效采样 fps，供播放器换算时间。
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

    // 渲染顺序：深度越大越靠后；同深度按网格下标稳定排序。
    for (index, mesh) in compiled.meshes.iter_mut().enumerate() {
        mesh.render_order = (((mesh.depth + 1.0) * 10000.0) as u32).saturating_add(index as u32);
    }
    if compiled.meshes.is_empty() {
        return Err("没有可转换的矢量图形（可能只包含图片/文本图层）".to_string());
    }
    Ok(compiled)
}

impl<'a> Compiler<'a> {
    /// 递归展开一个合成（顶层或预合成）的图层。
    ///
    /// `owner` 是承载该合成的预合成图层下标；`owner_ref` 为继承的参考帧。
    /// 每个图层分配一条 16 float × 帧数 的世界矩阵轨迹，并按 `ind` 建立
    /// 父子关系（`parent`）。
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
            // 深度按图层在合成中的顺序均分：越靠前的图层越靠前（z 越大），
            // 子合成在其 owner 深度附近再压缩到 0.16 的范围。
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

    /// 按图层类型构建内容：4 形状层、1 纯色层、0 预合成层。
    ///
    /// 预合成会递归展开资源内的图层，并把参考帧换算到子合成时间
    /// （`(ref - st) / sr`），时间拉伸 `sr` 不能为 0。
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

    /// 构建形状图层的参考帧网格。
    ///
    /// 参考帧用与烘焙时相同的顶点跟随上下文求值，保证参考拓扑与动画中
    /// 真实出现的拓扑一致；每个 drawable 单独生成一个网格。
    fn build_shape_layer(&mut self, id: usize, layer: &'a Layer) {
        // 参考几何在图层局部时间采样，与属性求值保持一致。
        let frame = self.plans[id].ref_frame - layer.st;
        let follows = self.collect_follows(id, &layer.shapes);
        // 顶点跟随也是几何动画的来源之一。
        let geometry_animated = shape::geometry_animated(&layer.shapes)
            || follows.as_ref().map(|specs| !specs.map.is_empty()).unwrap_or(false);
        let style_animated = shape::style_animated(&layer.shapes);
        {
            let plan = &mut self.plans[id];
            plan.geometry_animated = geometry_animated;
            plan.style_animated = style_animated;
            plan.follows = follows.clone();
        }

        // 参考帧使用与烘焙时相同的顶点跟随上下文，保证拓扑一致。
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
            let Some(surface) = mesh::build_surface(&drawable, self.half, self.edge_radius, self.curve_spacing) else {
                self.plans[id].blueprints.push(None);
                continue;
            };
            // 最终颜色 alpha = 样式 alpha × 样式 opacity；图层透明度在烘焙期再乘。
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

    /// 构建纯色层：把矩形区域当作一个填充 drawable，颜色从 `#rrggbb` 解析。
    fn build_solid_layer(&mut self, id: usize, layer: &'a Layer) {
        let sw = layer.solid_width.max(1.0);
        let sh = layer.solid_height.max(1.0);
        let rgb = parse_hex(&layer.solid_color);
        let contour = Contour {
            // 纯色层局部坐标原点为左上角，尺寸单位为像素。
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
        let Some(surface) = mesh::build_surface(&drawable, self.half, self.edge_radius, self.curve_spacing) else {
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

    /// 收紧烘焙帧预算；只能在 [`Compiler::bake`] 之前调用（轨迹仍为占位 0）。
    fn resize_frames(&mut self, new_count: usize) {
        if new_count >= self.frame_count {
            return;
        }
        self.frame_count = new_count;
        for track in self.transforms.iter_mut() {
            track.resize(new_count * 16, 0.0);
        }
        for mesh in self.meshes.iter_mut() {
            // 逐帧顶点在 bake 阶段追加（不同拓扑桶的顶点数可能不同），
            // 这里只重置预分配轨迹，不按固定长度扩容。
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

    /// 遍历形状树，把「顶点跟随空对象」表达式解析为同一合成内的 plan id。
    ///
    /// 返回 `None` 表示该图层没有顶点跟随表达式；每个键是形状的索引路径，
    /// 值中的 `usize::MAX` 表示同名空对象未找到（运行时退回原位）。
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

    /// 标记采样区间内图层 alpha 会变化的 plan。
    ///
    /// 合成 owner 的 plan 下标总小于其子图层，因此单次正序遍历即可把
    /// 「父级透明度动画 / 父级时间伸缩」的动画状态传给子图层；图层在采样
    /// 区间内不完整可见时也视为动态（需要逐帧写 alpha）。
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

    /// 仅为颜色或图层 alpha 真正变化的网格分配逐帧颜色轨迹。
    ///
    /// 轨迹每帧 4 个 f32（RGBA）；常量颜色保留在 mesh record 的
    /// `static_color` 中，不占用轨迹空间。
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

    /// 逐帧烘焙：写出所有图层的世界矩阵、动态颜色与几何顶点。
    ///
    /// 第 i 帧根合成时间为 `sample_start + sample_span * i / frame_count`。
    /// 变换写入 16 float 列主序矩阵；几何变化时按“姿态锚点”选桶，把该帧顶点
    /// 追加到 `frame_positions`，并在 `frame_offsets` 中记录每帧起始的 float
    /// 下标（末尾再补一个总数作为结束哨兵）。最后把初始顶点补齐到所有桶中
    /// 的最大顶点数，保证播放时可直接按桶切换索引。
    ///
    /// 姿态锚点：把采样区间等分为若干段，每段起点都用当帧几何重新三角化。
    /// 只靠一套参考三角化套全程时，形状剧烈形变会让三角形反绕/重叠，
    /// 填充上出现裂缝或穿插；分段的索引拓扑能把这些误差限制在一小段内。
    fn bake(&mut self) {
        let frame_count = self.frame_count;
        let sample_start = self.sample_start;
        let sample_span = self.sample_span;
        let plan_count = self.plans.len();
        let mesh_plans: Vec<usize> = (0..plan_count)
            .filter(|&id| !self.plans[id].blueprints.is_empty())
            .collect();

        // 所有缓存只分配一次，每帧清零复用，避免逐帧重新分配。
        // world_cache 存最终世界矩阵，chain_cache 存父级链矩阵，
        // opacity_cache 存累计 alpha，frame_cache 存图层局部帧。
        let mut world_cache: Vec<Option<[f32; 16]>> = vec![None; plan_count];
        let mut chain_cache: Vec<Option<[f32; 16]>> = vec![None; plan_count];
        let mut opacity_cache: Vec<Option<f32>> = vec![None; plan_count];
        let mut frame_cache: Vec<Option<f32>> = vec![None; plan_count];
        let mut follow_positions: Vec<Option<[f32; 2]>> = vec![None; plan_count];
        let mut scratch = mesh::FrameScratch::default();

        // 姿态锚点：把采样区间按 1/7 等分，段起点用当帧几何重建三角化。
        // 基础桶（下标 0）在每段起点之外还负责首帧，故这里只排 1/7..6/7。
        let mut anchor_frames: Vec<usize> = Vec::new();
        for step in 1..7 {
            let frame = step * frame_count / 7;
            if frame > 0 && frame < frame_count && anchor_frames.last() != Some(&frame) {
                anchor_frames.push(frame);
            }
        }
        // 每个网格当前使用的桶下标，以及基础桶是否已按首帧姿态重建。
        let mut active_bucket = vec![0usize; self.meshes.len()];
        let mut pose_ready = vec![false; self.meshes.len()];

        for frame_index in 0..frame_count {
            // 当前帧是否落在某个姿态锚点上。
            let anchor_slot = anchor_frames.binary_search(&frame_index).ok();
            // 根合成时间按采样区间线性分摊到 frame_count 帧。
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
                // 每帧每位图层写 16 个 f32（列主序 4×4），轨迹按帧紧密排列。
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
                // 只动几何或只动样式时按需求值：纯样式动画跳过路径扁平化。
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
                        // 动态颜色：取当帧 drawable 的颜色与自身透明度，
                        // 再乘图层累计 alpha 写入 RGBA 轨迹。
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
                        // 静态颜色：把图层常量 alpha 折进 mesh record，
                        // 这样完全不需要颜色轨迹。
                        mesh.static_color = [
                            blueprint.color[0],
                            blueprint.color[1],
                            blueprint.color[2],
                            blueprint.color[3] * opacity,
                        ];
                    }

                    if geometry_animated {
                        // 默认沿用基础拓扑桶（下标 0）。
                        let start = mesh
                            .frame_positions
                            .as_ref()
                            .map(|track| track.len())
                            .unwrap_or(0);
                        let mesh_index = blueprint.mesh_index;
                        let mut bucket_index = active_bucket[mesh_index];
                        if let Some(drawable) = evaluated
                            .as_ref()
                            .and_then(|result| result.drawables.get(drawable_index))
                        {
                            let half = mesh.half_thickness;
                            let slots = shape::drawable_slots(drawable);
                            if !slots.is_empty() {
                                if !pose_ready[mesh_index] {
                                    // 首个有效帧：用当帧姿态重建基础桶，取代
                                    // 建立在别处（通常是动画中点）的参考三角化。
                                    let natural = mesh::plan_slots(&slots, self.curve_spacing);
                                    if let Some(surface) = mesh::build_surface_from_slots(
                                        drawable,
                                        &slots,
                                        natural,
                                        half,
                                        edge_radius,
                                    ) {
                                        mesh.replace_bucket(0, surface);
                                        bucket_index = 0;
                                        pose_ready[mesh_index] = true;
                                    }
                                } else if anchor_slot.is_some()
                                    && mesh.buckets.len() < mesh::MAX_BUCKETS
                                {
                                    // 姿态锚点：为这一段形变新建一套三角化。
                                    let natural = mesh::plan_slots(&slots, self.curve_spacing);
                                    if let Some(surface) = mesh::build_surface_from_slots(
                                        drawable,
                                        &slots,
                                        natural,
                                        half,
                                        edge_radius,
                                    ) {
                                        bucket_index = mesh.push_bucket(surface);
                                    }
                                } else if slots.len() != mesh.buckets[bucket_index].slots.len() {
                                    // 段内轮廓槽位数量发生变化（如 trim）：优先
                                    // 复用槽位数一致的桶，桶还有空位则新建一套。
                                    if let Some(position) = mesh
                                        .buckets
                                        .iter()
                                        .position(|bucket| bucket.slots.len() == slots.len())
                                    {
                                        bucket_index = position;
                                    } else if mesh.buckets.len() < mesh::MAX_BUCKETS {
                                        let natural =
                                            mesh::plan_slots(&slots, self.curve_spacing);
                                        if let Some(surface) = mesh::build_surface_from_slots(
                                            drawable,
                                            &slots,
                                            natural,
                                            half,
                                            edge_radius,
                                        ) {
                                            bucket_index = mesh.push_bucket(surface);
                                        }
                                    }
                                }

                                // 本帧顶点追加到轨迹尾部；写入失败则回滚长度。
                                let mut wrote = false;
                                let mut write_start = 0usize;
                                let (count, stride, bucket_plans) = {
                                    let bucket = &mesh.buckets[bucket_index];
                                    let stride = if bucket.point_count > 0 {
                                        (bucket.vertex_count / bucket.point_count) as usize
                                    } else {
                                        1
                                    };
                                    (
                                        bucket.vertex_count as usize * 3,
                                        stride,
                                        bucket.slots.as_slice(),
                                    )
                                };
                                if let Some(track) = mesh.frame_positions.as_mut() {
                                    write_start = track.len();
                                    track.resize(write_start + count, 0.0);
                                    wrote = mesh::write_frame_with_slots(
                                        drawable,
                                        &slots,
                                        bucket_plans,
                                        half,
                                        edge_radius,
                                        &mut track[write_start..],
                                        &mut scratch,
                                    );
                                    if !wrote {
                                        track.truncate(write_start);
                                    }
                                }
                                // 折叠检测：填充前盖若出现反绕/重叠（有符号面积和明显
                                // 小于绝对面积和），立即用当前姿态重建一套三角化并重写
                                // 本帧顶点，避免把裂缝留到下一帧。描边的环形带有符号
                                // 面积和接近 0，不参与该判定。
                                if wrote && mesh.kind == 0 && mesh.buckets.len() < mesh::MAX_BUCKETS {
                                    let ratio = match mesh.frame_positions.as_ref() {
                                        Some(track) => cap_fold_ratio(
                                            &mesh.buckets[bucket_index].indices,
                                            stride,
                                            &track[write_start..write_start + count],
                                        ),
                                        None => 1.0,
                                    };
                                    if ratio > FOLD_RATIO_THRESHOLD {
                                        let natural =
                                            mesh::plan_slots(&slots, self.curve_spacing);
                                        if let Some(surface) = mesh::build_surface_from_slots(
                                            drawable,
                                            &slots,
                                            natural,
                                            half,
                                            edge_radius,
                                        ) {
                                            let new_index = mesh.push_bucket(surface);
                                            let (new_count, new_plans) = {
                                                let bucket = &mesh.buckets[new_index];
                                                (
                                                    bucket.vertex_count as usize * 3,
                                                    bucket.slots.as_slice(),
                                                )
                                            };
                                            let mut rewritten = false;
                                            if let Some(track) = mesh.frame_positions.as_mut() {
                                                // 回滚旧拓扑写入的本帧顶点，再按新桶重写。
                                                track.truncate(write_start);
                                                track.resize(write_start + new_count, 0.0);
                                                rewritten = mesh::write_frame_with_slots(
                                                    drawable,
                                                    &slots,
                                                    new_plans,
                                                    half,
                                                    edge_radius,
                                                    &mut track[write_start..],
                                                    &mut scratch,
                                                );
                                                if !rewritten {
                                                    track.truncate(write_start);
                                                }
                                            }
                                            if rewritten {
                                                bucket_index = new_index;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // 记录本网格接下来继续使用的桶（无几何的帧沿用上一段）。
                        active_bucket[mesh_index] = bucket_index;
                        // 即使本帧没有有效 drawable，偏移表也要保持每帧一项。
                        // frame_offsets 记录 float 下标（不是顶点数），与
                        // frame_positions 的长度单位一致。
                        mesh.frame_offsets.push(start as u32);
                        mesh.frame_buckets.push(bucket_index as u8);
                    }
                }
            }
        }

        // 收尾：补齐偏移表并扩张初始顶点缓冲。
        // 每个桶可能比参考桶需要更多顶点，因此 positions 要按所有桶的最大
        // 顶点数对齐；未使用的尾部填 0。frame_offsets 再追加总长度作为
        // 最后一帧的结束哨兵（长度 = frame_count + 1）。
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

/// 计算某网格前盖三角形的折叠程度：`Σ|面积| / |Σ有符号面积|`。
///
/// `indices` 是挤出后的索引缓冲，前盖顶点都满足 `index % stride == 0`，
/// 用该条件把它们从侧墙/后盖三角形中筛出来；`frame` 是本帧顶点缓冲切片
/// （`[x, y, z]` 连续排列）。返回值 1.0 表示三角形绕向一致且互不重叠，
/// 大于 1 说明存在反绕/重叠，越大越严重。面积和接近 0（完全退化）时
/// 返回 1.0，避免除零把结果放大。
fn cap_fold_ratio(indices: &[u32], stride: usize, frame: &[f32]) -> f32 {
    if stride == 0 || frame.len() < 3 {
        return 1.0;
    }
    let mask = stride as u32;
    let mut abs_sum = 0.0f64;
    let mut signed_sum = 0.0f64;
    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        if a % mask != 0 || b % mask != 0 || c % mask != 0 {
            continue;
        }
        let va = a as usize * 3;
        let vb = b as usize * 3;
        let vc = c as usize * 3;
        if va + 2 >= frame.len() || vb + 2 >= frame.len() || vc + 2 >= frame.len() {
            continue;
        }
        // 2D 有符号面积（鞋带公式）；绕向一致时为同一符号。
        let area = ((frame[vb] - frame[va]) * (frame[vc + 1] - frame[va + 1])
            - (frame[vc] - frame[va]) * (frame[vb + 1] - frame[va + 1]))
            * 0.5;
        abs_sum += (area as f64).abs();
        signed_sum += area as f64;
    }
    if abs_sum < 1e-6 || signed_sum.abs() < 1e-6 {
        return 1.0;
    }
    (abs_sum / signed_sum.abs()) as f32
}

/* ------------------------------------------------------------------ */
/* 顶点跟随表达式                                                       */
/* ------------------------------------------------------------------ */

/// 在形状图层自身所在的合成内解析空对象名称，并记录形状的索引路径。
///
/// 递归进入 `gr` 组的子项；遇到带顶点跟随表达式的 `sh` 形状时，把目标空
/// 对象解析为同 owner 下同名图层的 plan id（未找到记为 `usize::MAX`），
/// 并以当前索引链 `path` 作为键写入 `specs.map`。
fn collect_follows_inner(
    plans: &[Plan],
    owner: Option<usize>,
    items: &[Value],
    path: &mut Vec<usize>,
    specs: &mut FollowSpecs,
) {
    // 未找到目标空对象时的占位 plan id。
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

/// 图层锚点在合成坐标系中的位置（父级链已烘焙）。
///
/// 返回 `[x, y]`（Lottie 像素）：用图层链矩阵 [`plan_chain`] 变换锚点
/// `a`，即 `fromCompToSurface` 反变换的目标位置。
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

/// 每帧把被引用的空对象锚点位置解析到 `out`，按 plan 下标索引。
///
/// 同一目标只计算一次；未被引用的位置保持 `None`。
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

/// 形状图层合成矩阵的逆（对应表达式 `fromCompToSurface`）。
///
/// 把合成坐标换算回图层局部坐标；矩阵不可逆时退回单位变换。
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
/* 矩阵工具                                                             */
/* ------------------------------------------------------------------ */

/// 全局归一化矩阵：把 Lottie 画布映射到 three.js 常用的世界尺寸。
///
/// 缩放系数为 `3.6 / max(width, height)`（three.js 中参考相机高度约 3.6
/// 单位）；Y 轴取负，完成屏幕坐标（Y 向下）到世界坐标（Y 向上）的翻转；
/// 平移把画布中心移到原点。返回 16 float 列主序 4×4 矩阵。
fn global_matrix(width: f32, height: f32) -> [f32; 16] {
    let size = width.max(height).max(1.0);
    let scale = 3.6 / size;
    // 列主序排列：[a, b, 0, 0, c, d, 0, 0, 0, 0, s, 0, tx, ty, 0, 1]。
    [
        scale, 0.0, 0.0, 0.0,
        0.0, -scale, 0.0, 0.0,
        0.0, 0.0, scale, 0.0,
        -scale * width * 0.5, scale * height * 0.5, 0.0, 1.0,
    ]
}

/// 4×4 列主序矩阵乘法 `out = a * b`（`b` 先作用）。
///
/// 与列向量约定一致：对 `p' = a * b * p`，先由 `b` 变换再由 `a` 变换；
/// 元素按 `out[col * 4 + row]` 存放。
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

/// 把 2D 仿射 [`Affine`] 提升为 4×4 列主序矩阵（Z 保持不变）。
fn affine_matrix(matrix: Affine) -> [f32; 16] {
    [
        matrix.a, matrix.b, 0.0, 0.0,
        matrix.c, matrix.d, 0.0, 0.0,
        0.0, 0.0, 1.0, 0.0,
        matrix.tx, matrix.ty, 0.0, 1.0,
    ]
}

/// 计算图层自身坐标系下的时间（单位：帧）。
///
/// 父合成按时间伸缩 `sr` 换算子图层时间（`renderedFrame = num / sr`），
/// lottie-web 的属性求值再相对图层起始时间 `st` 偏移。
fn plan_local_frame(plans: &[Plan], id: usize, root_frame: f32) -> f32 {
    let parent_frame = match plans[id].owner {
        Some(owner) => {
            // 父合成按时间伸缩 sr 换算子图层时间（renderedFrame = num / sr）。
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
    // lottie-web 的属性时间相对图层起始时间 st。
    parent_frame - plans[id].layer.st
}

/// 图层在给定合成时间（帧）是否可见；`ip == op` 视为始终可见。
fn visible(layer: &Layer, frame: f32) -> bool {
    if layer.op <= layer.ip {
        true
    } else {
        frame >= layer.ip && frame < layer.op
    }
}

/// 图层不透明度（0..1）：求值后钳制到 0..100 再除以 100。
fn layer_opacity(layer: &Layer, frame: f32) -> f32 {
    prop::eval_scalar(&layer.ks, "o", frame, 100.0).clamp(0.0, 100.0) / 100.0
}

/// 图层自身到父级链的累计仿射矩阵（不含全局归一化矩阵）。
///
/// 结果缓存在 `cache`；`frame_cache` 同时记录图层的局部帧，供上层复用。
/// 乘法顺序遵循列向量约定：先应用子图层局部变换，再应用父级链。
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
            // 父矩阵在左：p' = parent * local * p，即先局部后父级。
            let parent_matrix = plan_chain(plans, parent, root_frame, cache, frame_cache);
            mat_mul(&parent_matrix, &local)
        }
        None => local,
    };
    cache[id] = Some(matrix);
    frame_cache[id] = Some(frame);
    matrix
}

/// 图层最终世界矩阵与累计不透明度。
///
/// 世界矩阵 = 父级（owner 合成或全局归一化矩阵）* 图层链矩阵，并把图层
/// 深度写入 `world[14]` 完成深度烘焙；返回值第二项为 `0..1` 的累计 alpha
/// （父级与自身可见性相乘）。
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
    // 把图层深度烘焙进矩阵，播放器可直接上传，深度滑块只需再叠加增量。
    world[14] += plans[id].depth;
    let frame = frame_cache[id].unwrap_or_else(|| plan_local_frame(plans, id, root_frame));
    // ip / op 位于父合成时间轴，需要用「局部帧 + st」换回合成时间。
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

/// 解析 `#RRGGBB`（或 6 位十六进制）颜色为归一化 RGB（0..1）；
/// 解析失败时退回浅灰 `[0.8, 0.8, 0.8]`。
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

/// 本转换器支持的图层类型：0 预合成、1 纯色、4 形状。
pub fn is_supported_layer(layer: &Value) -> bool {
    matches!(
        layer.get("ty").and_then(Value::as_i64).unwrap_or(-1),
        0 | 1 | 4
    )
}
