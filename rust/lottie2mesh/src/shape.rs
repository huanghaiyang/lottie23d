//! 形状树求值：把 Lottie 的嵌套形状对象编译成单帧的平坦绘制项。
//!
//! 模块职责：
//! * 解析 `gr`（组）、`sh`（路径）、`rc` / `el` / `sr`（图元）、`tm`（trim）、
//!   `mm`（合并路径）、`fl` / `st`（填充/描边）等节点；
//! * 把组变换直接烘焙进轮廓坐标（`Affine`），播放期无需再做 2D 变换；
//! * 按图层解析 AE「顶点跟随空对象」表达式，把路径顶点替换为空对象位置；
//! * 检测几何/样式是否随时间变化，供编译器决定是否需要逐帧烘焙。
//!
//! 单位约定：轮廓坐标为 Lottie 像素；颜色各分量为 0..1；`opacity` 为
//! 0..1 的乘数；trim 的 start / end / offset 均已归一化为 0..1。
//! 只有在 `geometry = false` 的纯样式模式下才跳过路径扁平化，此时返回的
//! drawable 与几何模式保持相同顺序和数量。

use std::borrow::Cow;
use std::collections::HashMap;

use serde_json::Value;

use crate::boolops::{self, BoolOp};
use crate::path::{self, Contour};
use crate::prop;

/// 绘制类型：填充或描边。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrawKind {
    Fill,
    Stroke,
}

/// trim path 的归一化参数。
#[derive(Clone, Copy, Debug)]
pub struct TrimSpec {
    /// 归一化起点（0..1 弧长比例）。
    pub start: f32,
    /// 归一化终点（0..1 弧长比例）。
    pub end: f32,
    /// 归一化偏移（0..1，会环绕）。
    pub offset: f32,
    /// Lottie 的 trim 模式 `m`（当前未参与几何计算）。
    pub mode: i32,
}

/// 一个待绘制的轮廓，可附带 trim 信息。
#[derive(Clone, Debug)]
pub struct PathItem {
    pub contour: Contour,
    pub trim: Option<TrimSpec>,
}

/// 线性/径向渐变的求值规格。
///
/// 端点与 stop 都取自 Lottie 的 `gf` / `gs` 节点；端点坐标已随组变换烘焙，
/// 因此与三角化后的顶点处于同一坐标系，可直接按顶点位置取色。
#[derive(Clone, Debug)]
pub struct GradientSpec {
    /// 渐变起点（线性渐变的 0% / 径向渐变的圆心）。
    pub start: [f32; 2],
    /// 渐变终点（线性渐变的 100% / 径向渐变的半径端点）。
    pub end: [f32; 2],
    /// Lottie 的 `t`：1 线性、2 径向。
    pub kind: u8,
    /// `(偏移 0..1, RGB 0..1)`，已按偏移升序排列。
    pub stops: Vec<(f32, [f32; 3])>,
    /// `(偏移 0..1, alpha 0..1)`；为空表示不透明。
    /// Lottie 把透明度 stop 成对紧跟在颜色 stop 之后。
    pub alpha_stops: Vec<(f32, f32)>,
}

impl GradientSpec {
    /// 把 2D 位置换算成渐变参数 `t`（0..1）。
    ///
    /// 线性渐变把点投影到 start→end 轴；径向渐变用点到圆心的距离
    /// 除以半径。落在两端之外时自然被 clamp 到 0 / 1。
    pub fn param(&self, p: [f32; 2]) -> f32 {
        let dx = self.end[0] - self.start[0];
        let dy = self.end[1] - self.start[1];
        let raw = if self.kind == 2 {
            // 径向：半径为 0 时整体取首色。
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-6 {
                0.0
            } else {
                let px = p[0] - self.start[0];
                let py = p[1] - self.start[1];
                (px * px + py * py).sqrt() / len
            }
        } else {
            // 线性：在渐变轴上的投影比例。
            let len2 = dx * dx + dy * dy;
            if len2 < 1e-6 {
                0.0
            } else {
                ((p[0] - self.start[0]) * dx + (p[1] - self.start[1]) * dy) / len2
            }
        };
        raw.clamp(0.0, 1.0)
    }

    /// 按 2D 位置取渐变颜色（RGB，0..1）。
    pub fn color_at(&self, p: [f32; 2]) -> [f32; 3] {
        self.rgb_at(self.param(p))
    }

    /// 按 2D 位置取渐变透明度（0..1）；无透明度 stop 时为 1。
    pub fn alpha_at(&self, p: [f32; 2]) -> f32 {
        if self.alpha_stops.is_empty() {
            return 1.0;
        }
        self.alpha_at_t(self.param(p))
    }

    /// 按渐变参数取颜色，投影落在 stop 范围之外时取首/末 stop，
    /// 与 lottie-web 一致。
    fn rgb_at(&self, t: f32) -> [f32; 3] {
        if self.stops.is_empty() {
            return [1.0, 1.0, 1.0];
        }
        // stop 数量很少，直接线性扫描（无逐帧分配）。
        let mut previous = self.stops[0];
        if t <= previous.0 {
            return previous.1;
        }
        for stop in self.stops.iter().skip(1) {
            if t <= stop.0 {
                let span = (stop.0 - previous.0).max(1e-6);
                let k = (t - previous.0) / span;
                return [
                    previous.1[0] + (stop.1[0] - previous.1[0]) * k,
                    previous.1[1] + (stop.1[1] - previous.1[1]) * k,
                    previous.1[2] + (stop.1[2] - previous.1[2]) * k,
                ];
            }
            previous = *stop;
        }
        previous.1
    }

    /// 按渐变参数取透明度 stop 的插值。
    fn alpha_at_t(&self, t: f32) -> f32 {
        let mut previous = self.alpha_stops[0];
        if t <= previous.0 {
            return previous.1;
        }
        for stop in self.alpha_stops.iter().skip(1) {
            if t <= stop.0 {
                let span = (stop.0 - previous.0).max(1e-6);
                let k = (t - previous.0) / span;
                return previous.1 + (stop.1 - previous.1) * k;
            }
            previous = *stop;
        }
        previous.1
    }

    /// 所有 stop 颜色的平均值，用作无法逐顶点着色时的兜底色。
    pub fn average_rgb(&self) -> [f32; 3] {
        if self.stops.is_empty() {
            return [0.55, 0.62, 0.9];
        }
        let mut sum = [0.0f32; 3];
        for (_, rgb) in &self.stops {
            sum[0] += rgb[0];
            sum[1] += rgb[1];
            sum[2] += rgb[2];
        }
        let n = self.stops.len() as f32;
        [sum[0] / n, sum[1] / n, sum[2] / n]
    }
}

/// 一个填充/描边绘制项，持有它作用的所有路径。
#[derive(Clone, Debug)]
pub struct Drawable {
    pub kind: DrawKind,
    /// RGBA，RGB 与样式 alpha 均为 0..1。
    pub color: [f32; 4],
    /// 填充/描边样式的不透明度乘数（0..1）。
    pub opacity: f32,
    /// 描边宽度（像素）；填充为 0。
    pub width: f32,
    /// 线帽样式（Lottie `lc`）。
    pub cap: u8,
    /// 拐角样式（Lottie `lj`）。
    pub join: u8,
    /// 斜接限制倍数（Lottie `ml`）。
    pub miter: f32,
    pub name: String,
    /// 渐变规格；纯色填充/描边为 `None`。
    pub gradient: Option<GradientSpec>,
    pub paths: Vec<PathItem>,
}

/// 单帧形状求值结果：按绘制顺序排列的 drawable 列表。
pub struct ShapeResult {
    pub drawables: Vec<Drawable>,
}

/// 「顶点跟随空对象」的引用表。
///
/// key 为形状项在所属图层形状树内的下标路径（如 `[0, 2]` 表示第 0 组的
/// 第 2 个子节点），value 为对应空对象在图层的 plan id（按顶点顺序）。
#[derive(Default, Clone, Debug)]
pub struct FollowSpecs {
    pub map: HashMap<Vec<usize>, Vec<usize>>,
}

/// 逐帧求值「顶点跟随」表达式所需的上下文。
pub struct FollowContext<'a> {
    pub specs: &'a FollowSpecs,
    /// 形状图层合成矩阵的逆（对应表达式里的 `fromCompToSurface`）。
    pub layer_inverse: Affine,
    /// 每个图层 plan id 在合成空间的锚点位置。
    pub positions: &'a [Option<[f32; 2]>],
}

/// 2D 仿射变换，采用**列向量**约定：`p' = M * p`。
///
/// 矩阵布局为 `[[a, c, tx], [b, d, ty], [0, 0, 1]]`，即
/// `x' = a*x + c*y + tx`、`y' = b*x + d*y + ty`，与 lottie-web 的行向量
/// 矩阵互为转置，故组合顺序与旋转符号需要对应翻转。
#[derive(Clone, Copy, Debug)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Affine {
    /// 单位矩阵（不缩放、不旋转、不平移）。
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    /// 平移矩阵，单位为像素。
    pub fn translate(x: f32, y: f32) -> Self {
        Affine {
            tx: x,
            ty: y,
            ..Self::IDENTITY
        }
    }

    /// 缩放矩阵；`x` / `y` 为比例（1.0 = 原始大小）。
    pub fn scale(x: f32, y: f32) -> Self {
        Affine {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

    /// 旋转矩阵；`rad` 为弧度。列向量约定下等价于 lottie-web 的 `rotate(-r)`。
    pub fn rotate(rad: f32) -> Self {
        let (sin, cos) = rad.sin_cos();
        Affine {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// 水平错切矩阵；`x` 为错切量（`tan` 值），`c` 位置对应 x 受 y 影响。
    pub fn shear(x: f32) -> Self {
        Affine {
            a: 1.0,
            b: 0.0,
            c: x,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// 矩阵乘法 `self * other`：`other` 先作用，`self` 后作用。
    ///
    /// 与列向量约定一致，父变换应放在左侧（`parent.mul(child)`）。
    pub fn mul(self, other: Affine) -> Affine {
        Affine {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            tx: self.a * other.tx + self.c * other.ty + self.tx,
            ty: self.b * other.tx + self.d * other.ty + self.ty,
        }
    }

    /// 把点 `p`（像素）应用变换，返回新坐标。
    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        [
            self.a * p[0] + self.c * p[1] + self.tx,
            self.b * p[0] + self.d * p[1] + self.ty,
        ]
    }

    /// 求逆仿射；行列式接近 0（奇异矩阵）时返回 `None`。
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-9 {
            return None;
        }
        Some(Affine {
            a: self.d / det,
            b: -self.b / det,
            c: -self.c / det,
            d: self.a / det,
            tx: (self.c * self.ty - self.d * self.tx) / det,
            ty: (self.b * self.tx - self.a * self.ty) / det,
        })
    }
}

/// 求图层位置（像素）：支持整体位置或 `p.s === true` 的 x/y/z 分离属性。
///
/// lottie-web 对分离属性逐轴求值，本函数保持同样行为；返回 3 维以兼容
/// 带 `z` 的 3D 图层。
fn position_of(ks: &Value, frame: f32) -> [f32; 3] {
    if let Some(p) = ks.get("p") {
        if p.get("s").and_then(Value::as_bool).unwrap_or(false) {
            return [
                prop::eval_scalar(p, "x", frame, 0.0),
                prop::eval_scalar(p, "y", frame, 0.0),
                prop::eval_scalar(p, "z", frame, 0.0),
            ];
        }
        let value = prop::eval(p, frame);
        return [
            value.first().copied().unwrap_or(0.0),
            value.get(1).copied().unwrap_or(0.0),
            value.get(2).copied().unwrap_or(0.0),
        ];
    }
    [0.0, 0.0, 0.0]
}

/// 求 Z 轴旋转角（度）。
///
/// 3D 图层存在 `rx` 时优先用 `rz`（仅当 `rz` 也存在），否则退回二维 `r`，
/// 与 lottie-web 的 `TransformProperty` 一致。
fn rotation_of(ks: &Value, frame: f32) -> f32 {
    if ks.get("rx").is_some() {
        let rz = prop::eval_scalar(ks, "rz", frame, 0.0);
        if ks.get("rz").is_some() {
            return rz;
        }
    }
    prop::eval_scalar(ks, "r", frame, 0.0)
}

/// 根据 Lottie 的 `tr`（组变换）或图层的 `ks` 构造 2D 仿射矩阵。
///
/// 组合顺序与 lottie-web 一致：先平移，再旋转、错切、缩放，最后把锚点
/// 移回原点（即 `T(p) * R * Sk * S * T(-a)`）。`frame` 为局部帧号；
/// 位置/锚点单位为像素，缩放为百分比（100 = 原始大小），旋转为度。
pub fn transform_matrix(ks: &Value, frame: f32) -> Affine {
    transform_matrix_at(ks, frame, None, None)
}

/// 与 [`transform_matrix`] 相同，但允许用外部位置 / 旋转覆盖 `ks.p`、`ks.r`。
///
/// 用于复现位置与旋转表达式（如 `Trace Path`：图层位置由目标路径上的
/// 采样点决定，旋转取路径切线方向），此时 `ks.p` / `ks.r` 的静态值不再生效。
/// `position` 为合成/父级空间中的像素坐标，`rotation` 为角度（度）。
pub fn transform_matrix_at(
    ks: &Value,
    frame: f32,
    position: Option<[f32; 2]>,
    rotation: Option<f32>,
) -> Affine {
    let p = match position {
        Some(p) => [p[0], p[1], 0.0],
        None => position_of(ks, frame),
    };
    let a = prop::eval_in(ks, "a", frame, &[0.0, 0.0, 0.0]);
    let s = prop::eval_in(ks, "s", frame, &[100.0, 100.0, 100.0]);
    let r = rotation.unwrap_or_else(|| rotation_of(ks, frame));
    let sk = prop::eval_scalar(ks, "sk", frame, 0.0);
    let sa = prop::eval_scalar(ks, "sa", frame, 0.0);

    let mut m = Affine::translate(p[0], p.get(1).copied().unwrap_or(0.0));
    if r != 0.0 {
        // lottie-web 用行向量矩阵并调用 `rotate(-r)`；`Affine` 是转置后的
        // 列向量形式，因此旋转符号相应翻转为正。
        m = m.mul(Affine::rotate(r.to_radians()));
    }
    if sk != 0.0 {
        // 错切围绕 `sa` 指定的轴进行：先转到该轴，错切后再转回。
        let axis = sa.to_radians();
        let shear = Affine::rotate(sa.to_radians())
            .mul(Affine::shear((-sk).to_radians().tan()))
            .mul(Affine::rotate(-axis));
        m = m.mul(shear);
    }
    // Lottie 缩放是百分比，需要除以 100 归一化。
    let sx = s.first().copied().unwrap_or(100.0) / 100.0;
    let sy = s.get(1).copied().unwrap_or(100.0) / 100.0;
    if sx != 1.0 || sy != 1.0 {
        m = m.mul(Affine::scale(sx, sy));
    }
    // 锚点：把局部坐标平移使锚点落在变换后的位置上。
    m.mul(Affine::translate(
        -a.first().copied().unwrap_or(0.0),
        -a.get(1).copied().unwrap_or(0.0),
    ))
}

/// 求值形状树（几何模式），返回单帧的 drawable 列表。
pub fn eval_shapes(shapes: &[Value], frame: f32) -> ShapeResult {
    eval_with_geometry(shapes, frame, true, None)
}

/// 与 [`eval_shapes`] 相同，但额外传入「顶点跟随空对象」的求值上下文。
pub fn eval_shapes_follow(
    shapes: &[Value],
    frame: f32,
    ctx: &FollowContext<'_>,
) -> ShapeResult {
    eval_with_geometry(shapes, frame, true, Some(ctx))
}

/// 只求值颜色/线宽等样式，跳过路径扁平化与组变换。
///
/// 返回的 drawable 顺序与 [`eval_shapes`] 完全一致，因此样式动画可在不重建
/// 几何的前提下逐帧刷新颜色。
pub fn eval_styles(shapes: &[Value], frame: f32) -> ShapeResult {
    eval_with_geometry(shapes, frame, false, None)
}

/// 取形状树中第一个可见 `sh` 路径的原始贝塞尔数据，用于 `pointOnPath` 语义。
///
/// 与 [`eval_shapes`] 的遍历顺序一致（深度优先）；只叠加「顶点跟随空对象」
/// 的顶点替换，**不应用组变换**——lottie-web 的 `pointOnPath` 返回的是
/// 形状属性自身的坐标，组变换由渲染阶段单独处理。
pub fn eval_trace_path(
    shapes: &[Value],
    frame: f32,
    ctx: Option<&FollowContext<'_>>,
) -> Option<path::PathData> {
    eval_trace_path_inner(shapes, frame, ctx, &mut Vec::new())
}

fn eval_trace_path_inner(
    items: &[Value],
    frame: f32,
    ctx: Option<&FollowContext<'_>>,
    path: &mut Vec<usize>,
) -> Option<path::PathData> {
    for (item_index, item) in items.iter().enumerate() {
        if hidden(item) {
            continue;
        }
        match item.get("ty").and_then(Value::as_str).unwrap_or("") {
            "sh" => {
                let ks = item.get("ks")?;
                // 旧版 Bodymovin 把闭合标记放在 `sh.closed` 上；映射到路径本身的 `c`。
                let closed = item.get("closed").and_then(Value::as_bool);
                let mut data = path::eval_path_property(ks, frame, closed)?;
                if let Some(ctx) = ctx {
                    path.push(item_index);
                    if let Some(targets) = ctx.specs.map.get(path.as_slice()) {
                        for (index, target) in targets.iter().enumerate() {
                            if index >= data.v.len() {
                                break;
                            }
                            if let Some(position) = ctx.positions.get(*target).copied().flatten() {
                                data.v[index] = ctx.layer_inverse.apply(position);
                            }
                        }
                    }
                    path.pop();
                }
                return Some(data);
            }
            "gr" => {
                if let Some(children) = item.get("it").and_then(Value::as_array) {
                    path.push(item_index);
                    let found = eval_trace_path_inner(children, frame, ctx, path);
                    path.pop();
                    if found.is_some() {
                        return found;
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// 形状求值的统一实现；`geometry = false` 时只收集样式与路径计数。
fn eval_with_geometry(
    shapes: &[Value],
    frame: f32,
    geometry: bool,
    ctx: Option<&FollowContext<'_>>,
) -> ShapeResult {
    let mut result = GroupResult::default();
    let mut path = Vec::new();
    eval_group_items(shapes, frame, &mut result, geometry, ctx, &mut path, &[]);
    // 绘制顺序取反：AE 的形状树按「上层在前」存储，lottie-web 的
    // `searchShapes` 也是从末尾向前遍历再逐级 append，因此数组里越靠前的
    // 组/样式画得越晚（越在上）。这里对拍平后的绘制项整体取反即可等价还原；
    // 若按数组顺序直接绘制，靠后的样式会盖住靠前的样式（`dalek.json` 的
    // 拔塞器因此丢掉蓝色内圈）。
    result.drawables.reverse();
    ShapeResult {
        drawables: result.drawables,
    }
}

/// 单个组（或整棵树）的求值中间结果。
#[derive(Default)]
struct GroupResult {
    /// 本组内闭合的绘制项（已应用本组变换）。
    drawables: Vec<Drawable>,
    /// 尚未被样式消费的路径，向父组冒泡，供父级的填充/描边使用。
    unowned: Vec<PathItem>,
    /// 本组流出的路径总数；即使不生成几何也要保留，用于判断样式是否有效。
    path_count: usize,
}

/// 递归求值一组形状节点，结果写入 `out`。
///
/// `path` 是当前节点在形状树中的下标栈，用于匹配顶点跟随表达式；
/// `inherited_trims` 是祖先层传给本层的 trim 修饰器（按应用顺序）；
/// `geometry = false` 时只统计路径数量并跳过轮廓构造。
fn eval_group_items(
    items: &[Value],
    frame: f32,
    out: &mut GroupResult,
    geometry: bool,
    ctx: Option<&FollowContext<'_>>,
    path: &mut Vec<usize>,
    inherited_trims: &[TrimSpec],
) {
    // 组变换作用于组内全部内容，与 `tr` 在列表中的位置无关。
    let transform = items
        .iter()
        .find(|item| item.get("ty").and_then(Value::as_str) == Some("tr"))
        .map(|tr| transform_matrix(tr, frame))
        .unwrap_or(Affine::IDENTITY);

    // `tm` 按照 lottie-web 的修饰器语义作用：同一层里数组下标更大的
    // `tm` 修饰下标更小的路径（无论样式在两者之间还是之外），并递归
    // 作用于这些路径所在子组的全部内容。`shot_01` 的 `Trim Paths 1`
    // 正位于 `Shape 1` 组之外，旧实现只改本层 `active`，因此整条描边
    // 从不被修剪。
    let level_trims: Vec<(usize, TrimSpec)> = if geometry {
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.get("ty").and_then(Value::as_str) == Some("tm") && !hidden(item)
            })
            .map(|(index, item)| (index, eval_trim(item, frame)))
            .collect()
    } else {
        Vec::new()
    };

    let mut active: Vec<PathItem> = Vec::new();
    let mut active_count = 0usize;
    let mut local = GroupResult::default();

    for (item_index, item) in items.iter().enumerate() {
        let ty = item.get("ty").and_then(Value::as_str).unwrap_or("");
        match ty {
            "sh" => {
                if hidden(item) {
                    continue;
                }
                active_count += 1;
                if !geometry {
                    continue;
                }
                if let Some(ks) = item.get("ks") {
                    let closed = item.get("closed").and_then(Value::as_bool);
                    if let Some(shape_path) = path::eval_path_property(ks, frame, closed) {
                        let mut data = shape_path;
                        if let Some(ctx) = ctx {
                            // 顶点跟随：按形状树下标路径查表，把前面若干
                            // 顶点替换为空对象在合成空间的位置，再经
                            // `fromCompToSurface` 的逆变换回到图层局部空间。
                            path.push(item_index);
                            if let Some(targets) = ctx.specs.map.get(path.as_slice()) {
                                for (index, target) in targets.iter().enumerate() {
                                    if index >= data.v.len() {
                                        break;
                                    }
                                    if let Some(position) =
                                        ctx.positions.get(*target).copied().flatten()
                                    {
                                        data.v[index] = ctx.layer_inverse.apply(position);
                                    }
                                }
                            }
                            path.pop();
                        }
                        active.push(PathItem {
                            contour: path::flatten_path(&data),
                            trim: shape_trim(item_index, inherited_trims, &level_trims),
                        });
                    }
                }
            }
            "rc" => {
                if hidden(item) {
                    continue;
                }
                active_count += 1;
                if geometry {
                    active.push(PathItem {
                        contour: eval_rect(item, frame),
                        trim: shape_trim(item_index, inherited_trims, &level_trims),
                    });
                }
            }
            "el" => {
                if hidden(item) {
                    continue;
                }
                active_count += 1;
                if geometry {
                    active.push(PathItem {
                        contour: eval_ellipse(item, frame),
                        trim: shape_trim(item_index, inherited_trims, &level_trims),
                    });
                }
            }
            "sr" => {
                if hidden(item) {
                    continue;
                }
                active_count += 1;
                if geometry {
                    active.push(PathItem {
                        contour: eval_polystar(item, frame),
                        trim: shape_trim(item_index, inherited_trims, &level_trims),
                    });
                }
            }
            "tm" => {
                // trim path 已在创建路径时按 `level_trims` 一并应用，
                // 这里只需保留占位，避免重复改写已消费的路径。
            }
            "mm" => {
                // 合并路径：作用于本组内位于它上方（数组更靠前）的全部路径，
                // 包括子组未被消费而冒泡上来的路径。
                if hidden(item) || !geometry || active.len() < 2 {
                    continue;
                }
                let mode = item.get("mm").and_then(Value::as_i64).unwrap_or(1);
                let op = match mode {
                    3 => BoolOp::Subtract,
                    4 => BoolOp::Intersect,
                    5 => BoolOp::Exclude,
                    // 1 Merge / 2 Add 及未知取值统一按并集处理。
                    _ => BoolOp::Union,
                };
                let contours: Vec<Contour> =
                    active.iter().map(|path| path.contour.clone()).collect();
                let merged = boolops::combine(&contours, op);
                // 并集算空只可能来自退化几何，此时保留原路径更稳妥；
                // 相交/相减算空则代表区域确实被消掉，应如实清空。
                let empty_fallback = op == BoolOp::Union;
                if merged.is_empty() && empty_fallback {
                    continue;
                }
                // 合并结果是一条新路径，原有的 trim 信息不再适用。
                active = merged
                    .into_iter()
                    .map(|contour| PathItem {
                        contour,
                        trim: None,
                    })
                    .collect();
            }
            "fl" | "gf" => {
                if hidden(item) {
                    continue;
                }
                if active_count > 0 {
                    local.drawables.push(build_drawable(
                        DrawKind::Fill, item, frame, &active,
                    ));
                }
            }
            "st" | "gs" => {
                if hidden(item) {
                    continue;
                }
                if active_count > 0 {
                    local.drawables.push(build_drawable(
                        DrawKind::Stroke, item, frame, &active,
                    ));
                }
            }
            "gr" => {
                if hidden(item) {
                    continue;
                }
                if let Some(children) = item.get("it").and_then(Value::as_array) {
                    let mut child = GroupResult::default();
                    let child_trims = applicable_trims(inherited_trims, &level_trims, item_index);
                    path.push(item_index);
                    eval_group_items(
                        children,
                        frame,
                        &mut child,
                        geometry,
                        ctx,
                        path,
                        &child_trims,
                    );
                    path.pop();
                    local.drawables.extend(child.drawables);
                    // 子组未被消费的路径冒泡到当前组，路径计数同样累计。
                    active_count += child.path_count;
                    if geometry {
                        active.extend(child.unowned);
                    }
                }
            }
            _ => {}
        }
    }

    // 本组变换烘焙进几何：绘制项、冒泡路径、待消费路径都要变换。
    if geometry {
        for drawable in local.drawables.iter_mut() {
            for path in drawable.paths.iter_mut() {
                transform_contour(&mut path.contour, transform);
            }
            // 渐变端点与路径同属本组坐标系，必须一起变换，否则组内缩放/
            // 旋转后渐变会错位（lottie 的 `s` / `e` 本身不带 tr）。
            if let Some(gradient) = drawable.gradient.as_mut() {
                gradient.start = transform.apply(gradient.start);
                gradient.end = transform.apply(gradient.end);
            }
        }
        for path in local.unowned.iter_mut() {
            transform_contour(&mut path.contour, transform);
        }
        for path in active.iter_mut() {
            transform_contour(&mut path.contour, transform);
        }
    }

    out.drawables.extend(local.drawables);
    out.unowned.append(&mut active);
    out.path_count += active_count;
}

/// 用仿射矩阵就地变换轮廓的全部顶点。
fn transform_contour(contour: &mut Contour, matrix: Affine) {
    for point in contour.pts.iter_mut() {
        *point = matrix.apply(*point);
    }
}

/// 形状节点是否隐藏（`hd = true`）。
fn hidden(item: &Value) -> bool {
    item.get("hd").and_then(Value::as_bool).unwrap_or(false)
}

/// 求值一个 `tm` 节点：`s` / `e` 为百分比，`o` 为角度，统一归一化到 0..1。
fn eval_trim(item: &Value, frame: f32) -> TrimSpec {
    TrimSpec {
        start: prop::eval_scalar(item, "s", frame, 0.0) / 100.0,
        end: prop::eval_scalar(item, "e", frame, 100.0) / 100.0,
        offset: prop::eval_scalar(item, "o", frame, 0.0) / 360.0,
        mode: item.get("m").and_then(Value::as_i64).unwrap_or(1) as i32,
    }
}

/// 汇总某个节点实际生效的 trim 修饰器（按 lottie-web 的应用顺序）。
///
/// `level` 是当前层全部 `tm`（按下标升序）。修饰器在下标更大的位置注册，
/// 因此同层的生效顺序是下标从大到小；祖先层的修饰器注册得更早，排在前面。
fn applicable_trims(
    inherited: &[TrimSpec],
    level: &[(usize, TrimSpec)],
    index: usize,
) -> Vec<TrimSpec> {
    let mut out: Vec<TrimSpec> = inherited.to_vec();
    for (key, spec) in level.iter().rev() {
        if *key <= index {
            break;
        }
        out.push(*spec);
    }
    out
}

/// 单个形状路径最终携带的 trim。
///
/// 多层 trim 叠加需要把修剪结果再次展平，当前按最后应用的那一层处理
/// （现有资源里没有同时叠加多个 trim 的形状）。
fn shape_trim(
    index: usize,
    inherited: &[TrimSpec],
    level: &[(usize, TrimSpec)],
) -> Option<TrimSpec> {
    applicable_trims(inherited, level, index).last().copied()
}

/// 把颜色分量归一化到 0..1。
///
/// 旧版 Bodymovin 导出（`v` 低于 4.1.9，如 `dalek.json` 的 3.1.6）的 `fl` / `st`
/// 颜色用 0..255，新版用 0..1。lottie-web 在加载旧版文件时会把颜色除以 255；
/// 这里按数值判定（任一通道大于 1 即视为 0..255），否则颜色会被渲染端钳成白色。
fn normalize_color(color: &mut [f32; 4]) {
    if color.iter().any(|v| *v > 1.0) {
        for value in color.iter_mut() {
            *value /= 255.0;
        }
    }
}

/// 由填充/描边样式节点构造 `Drawable`，并把路径列表克隆进去。
///
/// 颜色分量、样式透明度均归一化为 0..1；描边宽度单位为像素。
fn build_drawable(kind: DrawKind, style: &Value, frame: f32, paths: &[PathItem]) -> Drawable {
    let is_gradient = style.get("ty").and_then(Value::as_str) == Some("gf")
        || style.get("ty").and_then(Value::as_str) == Some("gs");
    let spec = if is_gradient { gradient_spec(style, frame) } else { None };
    let (color, opacity) = if is_gradient {
        // 颜色只作为兜底（逐顶点渐变不可用时），取 stop 平均值。
        let opacity = prop::eval_scalar(style, "o", frame, 100.0) / 100.0;
        let rgb = spec.as_ref().map(GradientSpec::average_rgb).unwrap_or([0.55, 0.62, 0.9]);
        ([rgb[0], rgb[1], rgb[2], 1.0], opacity)
    } else {
        let c = prop::eval_in(style, "c", frame, &[0.0, 0.0, 0.0, 1.0]);
        let alpha = c.get(3).copied().unwrap_or(1.0);
        let opacity = prop::eval_scalar(style, "o", frame, 100.0) / 100.0;
        let mut rgba = [
            c.first().copied().unwrap_or(0.0),
            c.get(1).copied().unwrap_or(0.0),
            c.get(2).copied().unwrap_or(0.0),
            alpha,
        ];
        normalize_color(&mut rgba);
        (rgba, opacity)
    };

    let width = if kind == DrawKind::Stroke {
        prop::eval_scalar(style, "w", frame, 0.0)
    } else {
        0.0
    };

    Drawable {
        kind,
        color,
        opacity,
        width,
        cap: style.get("lc").and_then(Value::as_i64).unwrap_or(2) as u8,
        join: style.get("lj").and_then(Value::as_i64).unwrap_or(2) as u8,
        miter: style.get("ml").and_then(Value::as_f64).unwrap_or(4.0) as f32,
        name: style
            .get("nm")
            .and_then(Value::as_str)
            .unwrap_or("shape")
            .to_string(),
        gradient: spec,
        paths: paths.to_vec(),
    }
}

/// 解析 `gf` / `gs` 的渐变数据。
///
/// 颜色 stop 在 `g.k.k` 里按 `g.p × 4` 扁平存放（`偏移, r, g, b`），
/// 其后可能跟 `(偏移, alpha)` 的透明度 stop（本转换器暂不处理）。
/// 端点 `s` / `e` 与类型 `t` 是样式节点上的兄弟字段，属于形状组坐标系，
/// 组的 `tr` 会在 [`eval_group_items`] 末尾一并烘焙到端点里。
fn gradient_spec(style: &Value, frame: f32) -> Option<GradientSpec> {
    let g = style.get("g")?;
    let k = g.get("k").unwrap_or(g);
    // 动画渐变按当前帧求值，得到该帧的扁平 stop 数组。
    let flat: Vec<f32> = if k.get("a").and_then(Value::as_i64).unwrap_or(0) != 0 {
        crate::prop::eval(k, frame)
    } else {
        k.get("k")
            .unwrap_or(k)
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_f64)
                    .map(|value| value as f32)
                    .collect()
            })
            .unwrap_or_default()
    };
    let declared = g.get("p").and_then(Value::as_u64).unwrap_or(0) as usize;
    let stop_count = if declared > 0 { declared } else { flat.len() / 4 };
    let mut stops = Vec::with_capacity(stop_count);
    for index in 0..stop_count {
        let base = index * 4;
        if base + 3 >= flat.len() {
            break;
        }
        let offset = flat[base].clamp(0.0, 1.0);
        stops.push((
            offset,
            [
                flat[base + 1].clamp(0.0, 1.0),
                flat[base + 2].clamp(0.0, 1.0),
                flat[base + 3].clamp(0.0, 1.0),
            ],
        ));
    }
    if stops.is_empty() {
        return None;
    }
    stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    // 颜色 stop 之后的剩余数值是成对的 `(偏移, alpha)` 透明度 stop。
    let mut alpha_stops = Vec::new();
    let mut index = stop_count * 4;
    while index + 1 < flat.len() {
        alpha_stops.push((flat[index].clamp(0.0, 1.0), flat[index + 1].clamp(0.0, 1.0)));
        index += 2;
    }
    alpha_stops.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let start = prop::eval_in(style, "s", frame, &[0.0, 0.0]);
    let end = prop::eval_in(style, "e", frame, &[1.0, 0.0]);
    Some(GradientSpec {
        start: [start.first().copied().unwrap_or(0.0), start.get(1).copied().unwrap_or(0.0)],
        end: [end.first().copied().unwrap_or(1.0), end.get(1).copied().unwrap_or(0.0)],
        kind: style.get("t").and_then(Value::as_i64).unwrap_or(1) as u8,
        stops,
        alpha_stops,
    })
}

/// 求值 `rc` 矩形节点：位置、尺寸、圆角均以像素为单位，`d` 控制绕向。
fn eval_rect(item: &Value, frame: f32) -> Contour {
    let p = prop::eval_in(item, "p", frame, &[0.0, 0.0]);
    let s = prop::eval_in(item, "s", frame, &[0.0, 0.0]);
    let r = prop::eval_in(item, "r", frame, &[0.0, 0.0]);
    let direction = item.get("d").and_then(Value::as_i64).unwrap_or(1) as i32;
    let radius = if r.len() >= 2 {
        [r[0], r[1]]
    } else {
        [r.first().copied().unwrap_or(0.0); 2]
    };
    path::rect_contour(
        [p[0], p.get(1).copied().unwrap_or(0.0)],
        [s[0], s.get(1).copied().unwrap_or(0.0)],
        radius,
        direction,
    )
}

/// 求值 `el` 椭圆节点：位置与尺寸（像素）来自 `p` / `s`。
fn eval_ellipse(item: &Value, frame: f32) -> Contour {
    let p = prop::eval_in(item, "p", frame, &[0.0, 0.0]);
    let s = prop::eval_in(item, "s", frame, &[0.0, 0.0]);
    let direction = item.get("d").and_then(Value::as_i64).unwrap_or(1) as i32;
    path::ellipse_contour(
        [p[0], p.get(1).copied().unwrap_or(0.0)],
        [s[0], s.get(1).copied().unwrap_or(0.0)],
        direction,
    )
}

/// 求值 `sr` 多角星/多边形节点：`pt` 角数、`or` / `ir` 内外半径（像素）、
/// `r` 旋转（度），`sy == 1` 表示星形，`d` 控制绕向。
fn eval_polystar(item: &Value, frame: f32) -> Contour {
    let p = prop::eval_in(item, "p", frame, &[0.0, 0.0]);
    let points = prop::eval_scalar(item, "pt", frame, 5.0);
    let outer = prop::eval_scalar(item, "or", frame, 0.0);
    let inner = prop::eval_scalar(item, "ir", frame, 0.0);
    let rotation = prop::eval_scalar(item, "r", frame, 0.0);
    let star = item.get("sy").and_then(Value::as_i64).unwrap_or(1) == 1;
    let direction = item.get("d").and_then(Value::as_i64).unwrap_or(1) as i32;
    path::polystar_contour(
        [p[0], p.get(1).copied().unwrap_or(0.0)],
        points,
        outer,
        inner,
        rotation,
        star,
        direction,
    )
}

/* ------------------------------------------------------------------ */
/* Slots & animation detection                                         */
/* ------------------------------------------------------------------ */

/// 把 drawable 的路径展开为固定拓扑槽位。
///
/// 返回 `(contour, closed, is_trimmed)`：未 trim 的轮廓直接借用原缓冲
/// （`Cow::Borrowed`），避免逐帧复制；trim 过的轮廓则按
/// [`path::trim_slots`] 固定展开为两个槽位，第二槽即使为空也保留占位，
/// 从而保证所有帧的槽位数量一致。
pub fn drawable_slots(drawable: &Drawable) -> Vec<(Cow<'_, Contour>, bool, bool)> {
    let mut slots = Vec::with_capacity(drawable.paths.len());
    for item in &drawable.paths {
        match item.trim {
            Some(trim) => {
                let ranges = path::trim_slots(trim.start, trim.end, trim.offset);
                let _ = trim.mode;
                for (index, (start, length)) in ranges.iter().enumerate() {
                    if index > 0 && *length <= 1e-5 {
                        continue;
                    }
                    let contour = path::trim_segment(&item.contour, *start, *length);
                    // 只有截取长度接近整圈时，才把该槽位视为闭合轮廓。
                    let closed = item.contour.closed && *length >= 0.999;
                    slots.push((Cow::Owned(contour), closed, true));
                }
                // 第二个槽位为空时仍补一个单点退化轮廓，保持槽位计数稳定。
                if ranges[1].1 <= 1e-5 {
                    let origin = item.contour.pts.first().copied().unwrap_or([0.0, 0.0]);
                    slots.push((
                        Cow::Owned(Contour {
                            pts: vec![origin],
                            closed: false,
                        }),
                        false,
                        true,
                    ));
                }
            }
            None => slots.push((Cow::Borrowed(&item.contour), item.contour.closed, false)),
        }
    }
    slots
}

/// 判断属性对象是否为动画。
///
/// 判定规则与 `prop::is_keyframed` 一致：看 `k` 的首元素是否为关键帧对象，
/// 不依赖可能缺失的 `a` 字段（部分导出文件省掉了 `a`）。
pub fn is_animated(value: &Value) -> bool {
    crate::prop::is_keyframed(value)
}

/// 识别 AE「顶点跟随空对象」路径表达式，并按顶点顺序返回空对象名称。
///
/// 表达式形如
/// `origPoints[i] = fromCompToSurface(nullLayer.toComp(nullLayer.anchorPoint))`，
/// 即用空对象在合成空间的位置替换路径每个顶点。识别条件是同时包含
/// `fromCompToSurface` 与 `Layer Control`，并从 `nullLayerNames` 数组取名字。
/// 不是 `sh` 节点或不匹配表达式时返回 `None`。
pub fn vertex_follow_names(item: &Value) -> Option<Vec<String>> {
    if item.get("ty").and_then(Value::as_str) != Some("sh") {
        return None;
    }
    let expression = item.get("ks")?.get("x")?.as_str()?;
    if !expression.contains("fromCompToSurface") || !expression.contains("Layer Control") {
        return None;
    }
    let start = expression.find("nullLayerNames")?;
    let tail = &expression[start..];
    let array_start = tail.find('[')?;
    // 图层名可能自带方括号（如 "Path 1 [1.0]"），因此不能找第一个 `]`，
    // 而要用括号深度配对，定位 `nullLayerNames` 数组的结束位置。
    let mut depth = 0usize;
    let mut array_end = None;
    for (offset, ch) in tail[array_start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    array_end = Some(array_start + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let array_end = array_end?;
    if array_end <= array_start {
        return None;
    }
    let names = quoted_strings(&tail[array_start + 1..array_end]);
    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

/// 按源码顺序收集单引号或双引号字符串字面量（不含引号本身）。
/// 支持字符串内部出现另一种引号，遇到匹配的闭引号才提交。
fn quoted_strings(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<(char, String)> = None;
    for ch in source.chars() {
        match &mut current {
            Some((quote, text)) => {
                if ch == *quote {
                    out.push(std::mem::take(text));
                    current = None;
                } else {
                    text.push(ch);
                }
            }
            None => {
                if ch == '\'' || ch == '"' {
                    current = Some((ch, String::new()));
                }
            }
        }
    }
    out
}

/// 判断形状几何（路径、图元、trim、线宽、组变换）是否随时间变化。
///
/// 仅颜色/透明度动画不算几何动画；`tm` 一律视为几何动画，
/// 顶点跟随表达式也视为几何动画。
pub fn geometry_animated(shapes: &[Value]) -> bool {
    for item in shapes {
        let ty = item.get("ty").and_then(Value::as_str).unwrap_or("");
        match ty {
            "sh" => {
                if item.get("ks").map(is_animated).unwrap_or(false) {
                    return true;
                }
                if vertex_follow_names(item).is_some() {
                    return true;
                }
            }
            "rc" | "el" => {
                for key in ["p", "s", "r"] {
                    if item.get(key).map(is_animated).unwrap_or(false) {
                        return true;
                    }
                }
            }
            "sr" => {
                for key in ["p", "pt", "or", "os", "ir", "is", "r"] {
                    if item.get(key).map(is_animated).unwrap_or(false) {
                        return true;
                    }
                }
            }
            "tm" => return true,
            "st" => {
                if item.get("w").map(is_animated).unwrap_or(false) {
                    return true;
                }
            }
            "tr" => {
                for key in ["p", "a", "s", "r", "sk", "sa"] {
                    if item.get(key).map(is_animated).unwrap_or(false) {
                        return true;
                    }
                }
            }
            "gr" | "rp" => {
                if let Some(children) = item.get("it").and_then(Value::as_array) {
                    if geometry_animated(children) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// 判断填充/描边颜色或透明度是否有动画关键帧。
pub fn style_animated(shapes: &[Value]) -> bool {
    for item in shapes {
        let ty = item.get("ty").and_then(Value::as_str).unwrap_or("");
        match ty {
            "fl" | "st" | "gf" | "gs" => {
                for key in ["c", "o", "g"] {
                    if item.get(key).map(is_animated).unwrap_or(false) {
                        return true;
                    }
                }
            }
            "gr" | "rp" => {
                if let Some(children) = item.get("it").and_then(Value::as_array) {
                    if style_animated(children) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/* ------------------------------------------------------------------ */
/* 图层遮罩                                                            */
/* ------------------------------------------------------------------ */

/// 图层遮罩是否需要逐帧裁剪几何。
///
/// 目前只支持「单条遮罩 + 加法模式（`mode = "a"`）+ 未反相 + 凸轮廓」，
/// 且遮罩路径带关键帧的情况（例如海星眼睛的眨眼遮罩：矩形上下滑动）。
/// 形状层的静态遮罩仍按旧行为忽略；纯色层则连静态遮罩一起裁剪（见
/// [`mask_supports_layer`]），因为整块纯色矩形不裁会盖住整个画面。
pub fn mask_clips_geometry(masks: &[Value]) -> bool {
    mask_supports_layer(masks) && masks[0].get("pt").map(is_animated).unwrap_or(false)
}

/// 图层是否带一条本模块支持的遮罩（不论静态还是带关键帧）。
///
/// 纯色层（如 `dalek.json` 的 `tardisLigth`：1920×1080 近黑纯色块被一条静态
/// 遮罩裁成光束）必须按遮罩裁剪，否则整块矩形会盖住画面；这里只判断“能否
/// 处理”，是否逐帧重建几何由 [`mask_clips_geometry`] 决定。
pub fn mask_supports_layer(masks: &[Value]) -> bool {
    masks.len() == 1 && mask_supported(&masks[0])
}

/// 按遮罩裁剪当帧的 drawable 轮廓；不支持或求值失败时原样返回。
///
/// 被完全裁掉的路径会被丢弃；若某个 drawable 的路径全部被裁掉，
/// 该帧就写不出几何（播放端表现为该网格当帧不可见）。
pub fn apply_mask(result: &mut ShapeResult, masks: &[Value], frame: f32) {
    let Some(outline) = mask_outline(masks, frame) else {
        return;
    };
    for drawable in &mut result.drawables {
        let mut kept = Vec::with_capacity(drawable.paths.len());
        for item in std::mem::take(&mut drawable.paths) {
            let clipped = clip_convex_polygon(&item.contour.pts, &outline.pts);
            if clipped.len() < 3 {
                continue;
            }
            kept.push(PathItem {
                contour: Contour {
                    pts: clipped,
                    closed: item.contour.closed,
                },
                trim: item.trim,
            });
        }
        drawable.paths = kept;
    }
}

/// 遮罩是否属于本模块支持的子集（单条、加法、未反相）。
fn mask_supported(mask: &Value) -> bool {
    if mask.get("inv").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    mask.get("mode").and_then(Value::as_str).unwrap_or("a") == "a"
}

/// 求值遮罩路径并展平为闭合轮廓；允许多边形凹陷（纯色层直接拿它当轮廓）。
///
/// 遮罩按定义是闭合区域，因此显式传 `closed = Some(true)`，避免旧文件缺少
/// 路径内部的 `c` 标记时被当成开口折线。
pub fn mask_contour(masks: &[Value], frame: f32) -> Option<Contour> {
    if masks.len() != 1 || !mask_supported(&masks[0]) {
        return None;
    }
    let data = path::eval_path_property(masks[0].get("pt")?, frame, Some(true))?;
    let mut contour = path::flatten_path(&data);
    contour.closed = true;
    if contour.pts.len() < 3 {
        return None;
    }
    Some(contour)
}

/// 求值遮罩路径并展平为凸多边形；不可用（多条、非凸、退化）时返回 `None`。
fn mask_outline(masks: &[Value], frame: f32) -> Option<Contour> {
    let contour = mask_contour(masks, frame)?;
    if !is_convex(&contour.pts) {
        return None;
    }
    Some(contour)
}

/// 多边形有向面积（顶点顺序决定符号）。
fn polygon_area(pts: &[[f32; 2]]) -> f32 {
    let mut sum = 0.0;
    for index in 0..pts.len() {
        let a = pts[index];
        let b = pts[(index + 1) % pts.len()];
        sum += a[0] * b[1] - b[0] * a[1];
    }
    sum * 0.5
}

/// 是否凸多边形；允许极小凹量，避免浮点误差把矩形判成凹。
fn is_convex(pts: &[[f32; 2]]) -> bool {
    let count = pts.len();
    if count < 3 {
        return false;
    }
    let mut sign = 0.0f32;
    for index in 0..count {
        let a = pts[index];
        let b = pts[(index + 1) % count];
        let c = pts[(index + 2) % count];
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross.abs() < 1e-6 {
            continue;
        }
        let current = cross.signum();
        if sign == 0.0 {
            sign = current;
        } else if current != sign {
            return false;
        }
    }
    sign != 0.0
}

/// 两条线段的交点参数解（不限制在段内；调用方只在线段跨边界时使用）。
fn edge_intersection(p1: [f32; 2], p2: [f32; 2], p3: [f32; 2], p4: [f32; 2]) -> Option<[f32; 2]> {
    let denominator = (p2[0] - p1[0]) * (p4[1] - p3[1]) - (p2[1] - p1[1]) * (p4[0] - p3[0]);
    if denominator.abs() < 1e-9 {
        return None;
    }
    let t = ((p3[0] - p1[0]) * (p4[1] - p3[1]) - (p3[1] - p1[1]) * (p4[0] - p3[0])) / denominator;
    Some([p1[0] + t * (p2[0] - p1[0]), p1[1] + t * (p2[1] - p1[1])])
}

/// Sutherland–Hodgman：用凸多边形裁剪任意多边形，返回交集轮廓。
fn clip_convex_polygon(subject: &[[f32; 2]], clip: &[[f32; 2]]) -> Vec<[f32; 2]> {
    let mut clip_points: Vec<[f32; 2]> = clip.to_vec();
    // 统一为逆时针，使「内部」等价于裁剪边左侧。
    if polygon_area(&clip_points) < 0.0 {
        clip_points.reverse();
    }
    let mut output: Vec<[f32; 2]> = subject.to_vec();
    for index in 0..clip_points.len() {
        if output.len() < 3 {
            return Vec::new();
        }
        let a = clip_points[index];
        let b = clip_points[(index + 1) % clip_points.len()];
        let inside = |p: [f32; 2]| {
            (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-6
        };
        let mut next: Vec<[f32; 2]> = Vec::with_capacity(output.len() + 2);
        for vertex in 0..output.len() {
            let current = output[vertex];
            let previous = output[(vertex + output.len() - 1) % output.len()];
            let (current_in, previous_in) = (inside(current), inside(previous));
            if current_in != previous_in {
                if let Some(point) = edge_intersection(previous, current, a, b) {
                    next.push(point);
                }
            }
            if current_in {
                next.push(current);
            }
        }
        output = next;
    }
    // 去掉相邻重复点（裁剪边界恰好穿过顶点时会产生），并保证首尾不重复。
    output.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5);
    if output.len() >= 2 {
        let first = output[0];
        let last = output[output.len() - 1];
        if (first[0] - last[0]).abs() < 1e-5 && (first[1] - last[1]).abs() < 1e-5 {
            output.pop();
        }
    }
    if output.len() < 3 {
        return Vec::new();
    }
    output
}
