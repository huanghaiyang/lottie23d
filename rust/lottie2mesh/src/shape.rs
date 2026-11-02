//! 形状树求值：把 Lottie 的嵌套形状对象编译成单帧的平坦绘制项。
//!
//! 模块职责：
//! * 解析 `gr`（组）、`sh`（路径）、`rc` / `el` / `sr`（图元）、`tm`（trim）、
//!   `fl` / `st`（填充/描边）等节点；
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
    let p = position_of(ks, frame);
    let a = prop::eval_in(ks, "a", frame, &[0.0, 0.0, 0.0]);
    let s = prop::eval_in(ks, "s", frame, &[100.0, 100.0, 100.0]);
    let r = rotation_of(ks, frame);
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

/// 形状求值的统一实现；`geometry = false` 时只收集样式与路径计数。
fn eval_with_geometry(
    shapes: &[Value],
    frame: f32,
    geometry: bool,
    ctx: Option<&FollowContext<'_>>,
) -> ShapeResult {
    let mut result = GroupResult::default();
    let mut path = Vec::new();
    eval_group_items(shapes, frame, &mut result, geometry, ctx, &mut path);
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
/// `geometry = false` 时只统计路径数量并跳过轮廓构造。
fn eval_group_items(
    items: &[Value],
    frame: f32,
    out: &mut GroupResult,
    geometry: bool,
    ctx: Option<&FollowContext<'_>>,
    path: &mut Vec<usize>,
) {
    // 组变换作用于组内全部内容，与 `tr` 在列表中的位置无关。
    let transform = items
        .iter()
        .find(|item| item.get("ty").and_then(Value::as_str) == Some("tr"))
        .map(|tr| transform_matrix(tr, frame))
        .unwrap_or(Affine::IDENTITY);

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
                    if let Some(shape_path) = path::eval_path_property(ks, frame) {
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
                            trim: None,
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
                        trim: None,
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
                        trim: None,
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
                        trim: None,
                    });
                }
            }
            "tm" => {
                // trim path：s / e 为百分比，o 为角度；统一归一化到 0..1。
                if !geometry {
                    continue;
                }
                let s = prop::eval_scalar(item, "s", frame, 0.0) / 100.0;
                let e = prop::eval_scalar(item, "e", frame, 100.0) / 100.0;
                let o = prop::eval_scalar(item, "o", frame, 0.0) / 360.0;
                let spec = TrimSpec {
                    start: s,
                    end: e,
                    offset: o,
                    mode: item.get("m").and_then(Value::as_i64).unwrap_or(1) as i32,
                };
                // trim 只作用于当前组内已出现、尚未被消费的路径。
                for path in active.iter_mut() {
                    path.trim = Some(spec);
                }
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
                    path.push(item_index);
                    eval_group_items(children, frame, &mut child, geometry, ctx, path);
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

/// 由填充/描边样式节点构造 `Drawable`，并把路径列表克隆进去。
///
/// 颜色分量、样式透明度均归一化为 0..1；描边宽度单位为像素。
fn build_drawable(kind: DrawKind, style: &Value, frame: f32, paths: &[PathItem]) -> Drawable {
    let (color, opacity) = if style.get("ty").and_then(Value::as_str) == Some("gf")
        || style.get("ty").and_then(Value::as_str) == Some("gs")
    {
        gradient_color(style, frame)
    } else {
        let c = prop::eval_in(style, "c", frame, &[0.0, 0.0, 0.0, 1.0]);
        let alpha = c.get(3).copied().unwrap_or(1.0);
        let opacity = prop::eval_scalar(style, "o", frame, 100.0) / 100.0;
        (
            [
                c.first().copied().unwrap_or(0.0),
                c.get(1).copied().unwrap_or(0.0),
                c.get(2).copied().unwrap_or(0.0),
                alpha,
            ],
            opacity,
        )
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
        paths: paths.to_vec(),
    }
}

/// 用多个颜色 stop 的平均值近似填充渐变。
///
/// 几何、深度和动画仍来自精确的矢量路径，只有颜色被简化；返回 RGBA 与
/// 样式透明度（均 0..1）。渐变缺少有效 stop 时退回偏蓝的默认色。
fn gradient_color(style: &Value, frame: f32) -> ([f32; 4], f32) {
    let opacity = prop::eval_scalar(style, "o", frame, 100.0) / 100.0;
    let mut rgb = [0.55f32, 0.62, 0.9];
    let gradient = style.get("g");
    if let Some(g) = gradient {
        let k = g.get("k").unwrap_or(g);
        let value = if k.get("a").and_then(Value::as_i64).unwrap_or(0) != 0 {
            crate::prop::eval(k, frame)
        } else {
            let stops = k.get("k").unwrap_or(k);
            let mut sum = [0.0f32; 3];
            let mut count = 0.0f32;
            if let Some(list) = stops.as_array() {
                for stop in list {
                    if let Some(s) = stop.get("s").and_then(Value::as_array) {
                        if s.len() >= 3 {
                            sum[0] += s[0].as_f64().unwrap_or(0.0) as f32;
                            sum[1] += s[1].as_f64().unwrap_or(0.0) as f32;
                            sum[2] += s[2].as_f64().unwrap_or(0.0) as f32;
                            count += 1.0;
                        }
                    }
                }
            }
            if count > 0.0 {
                vec![sum[0] / count, sum[1] / count, sum[2] / count]
            } else {
                Vec::new()
            }
        };
        if value.len() >= 3 {
            rgb = [value[0], value[1], value[2]];
        }
    }
    ([rgb[0], rgb[1], rgb[2], 1.0], opacity)
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

/// 判断属性对象是否为动画（`a != 0`）。
pub fn is_animated(value: &Value) -> bool {
    value.get("a").and_then(Value::as_i64).unwrap_or(0) != 0
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
