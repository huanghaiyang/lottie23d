//! Shape tree evaluation.
//!
//! Converts the nested Lottie shape objects (`gr`, `sh`, `rc`, `el`, `sr`,
//! `tm`, `fl`, `st`) into a flat list of drawables for a single frame.
//! Group transforms are baked into contour coordinates, which keeps the
//! runtime animation work inside Rust.

use std::borrow::Cow;

use serde_json::Value;

use crate::path::{self, Contour};
use crate::prop;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DrawKind {
    Fill,
    Stroke,
}

#[derive(Clone, Copy, Debug)]
pub struct TrimSpec {
    /// Normalised start / end (0..1).
    pub start: f32,
    pub end: f32,
    /// Normalised offset (0..1).
    pub offset: f32,
    pub mode: i32,
}

#[derive(Clone, Debug)]
pub struct PathItem {
    pub contour: Contour,
    pub trim: Option<TrimSpec>,
}

#[derive(Clone, Debug)]
pub struct Drawable {
    pub kind: DrawKind,
    /// RGB + style alpha (0..1).
    pub color: [f32; 4],
    /// Fill/stroke opacity as a multiplier (0..1).
    pub opacity: f32,
    pub width: f32,
    pub cap: u8,
    pub join: u8,
    pub miter: f32,
    pub name: String,
    pub paths: Vec<PathItem>,
}

pub struct ShapeResult {
    pub drawables: Vec<Drawable>,
}

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
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    pub fn translate(x: f32, y: f32) -> Self {
        Affine {
            tx: x,
            ty: y,
            ..Self::IDENTITY
        }
    }

    pub fn scale(x: f32, y: f32) -> Self {
        Affine {
            a: x,
            d: y,
            ..Self::IDENTITY
        }
    }

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

    /// `self * other` — `other` is applied first.
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

    pub fn apply(&self, p: [f32; 2]) -> [f32; 2] {
        [
            self.a * p[0] + self.c * p[1] + self.tx,
            self.b * p[0] + self.d * p[1] + self.ty,
        ]
    }
}

/// Position may be a single property or split into `x` / `y` / `z`
/// (`p.s === true`), which lottie-web evaluates separately.
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

/// 3D layers rotate around Z via `rz` even when `r` is present, matching
/// lottie-web's `TransformProperty`.
fn rotation_of(ks: &Value, frame: f32) -> f32 {
    if ks.get("rx").is_some() {
        let rz = prop::eval_scalar(ks, "rz", frame, 0.0);
        if ks.get("rz").is_some() {
            return rz;
        }
    }
    prop::eval_scalar(ks, "r", frame, 0.0)
}

/// Build the 2D transform described by a Lottie `tr` group / layer transform.
pub fn transform_matrix(ks: &Value, frame: f32) -> Affine {
    let p = position_of(ks, frame);
    let a = prop::eval_in(ks, "a", frame, &[0.0, 0.0, 0.0]);
    let s = prop::eval_in(ks, "s", frame, &[100.0, 100.0, 100.0]);
    let r = rotation_of(ks, frame);
    let sk = prop::eval_scalar(ks, "sk", frame, 0.0);
    let sa = prop::eval_scalar(ks, "sa", frame, 0.0);

    let mut m = Affine::translate(p[0], p.get(1).copied().unwrap_or(0.0));
    if r != 0.0 {
        m = m.mul(Affine::rotate(-r.to_radians()));
    }
    if sk != 0.0 {
        let axis = sa.to_radians();
        let shear = Affine::rotate(sa.to_radians())
            .mul(Affine::shear((-sk).to_radians().tan()))
            .mul(Affine::rotate(-axis));
        m = m.mul(shear);
    }
    let sx = s.first().copied().unwrap_or(100.0) / 100.0;
    let sy = s.get(1).copied().unwrap_or(100.0) / 100.0;
    if sx != 1.0 || sy != 1.0 {
        m = m.mul(Affine::scale(sx, sy));
    }
    m.mul(Affine::translate(
        -a.first().copied().unwrap_or(0.0),
        -a.get(1).copied().unwrap_or(0.0),
    ))
}

pub fn eval_shapes(shapes: &[Value], frame: f32) -> ShapeResult {
    eval_with_geometry(shapes, frame, true)
}

/// Evaluate colours only, skipping path flattening and group transforms.
/// The returned drawables keep the same order as [`eval_shapes`], which lets
/// style-only animations avoid rebuilding geometry every frame.
pub fn eval_styles(shapes: &[Value], frame: f32) -> ShapeResult {
    eval_with_geometry(shapes, frame, false)
}

fn eval_with_geometry(shapes: &[Value], frame: f32, geometry: bool) -> ShapeResult {
    let mut result = GroupResult::default();
    eval_group_items(shapes, frame, &mut result, geometry);
    ShapeResult {
        drawables: result.drawables,
    }
}

#[derive(Default)]
struct GroupResult {
    drawables: Vec<Drawable>,
    unowned: Vec<PathItem>,
    /// Number of paths flowing out of this group (kept even without geometry).
    path_count: usize,
}

fn eval_group_items(items: &[Value], frame: f32, out: &mut GroupResult, geometry: bool) {
    // A group transform applies to all of its contents, regardless of where
    // the `tr` item sits in the list.
    let transform = items
        .iter()
        .find(|item| item.get("ty").and_then(Value::as_str) == Some("tr"))
        .map(|tr| transform_matrix(tr, frame))
        .unwrap_or(Affine::IDENTITY);

    let mut active: Vec<PathItem> = Vec::new();
    let mut active_count = 0usize;
    let mut local = GroupResult::default();

    for item in items {
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
                    if let Some(path) = path::eval_path_property(ks, frame) {
                        active.push(PathItem {
                            contour: path::flatten_path(&path),
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
                    eval_group_items(children, frame, &mut child, geometry);
                    local.drawables.extend(child.drawables);
                    active_count += child.path_count;
                    if geometry {
                        active.extend(child.unowned);
                    }
                }
            }
            _ => {}
        }
    }

    // Apply this group's transform to the geometry produced inside it.
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

fn transform_contour(contour: &mut Contour, matrix: Affine) {
    for point in contour.pts.iter_mut() {
        *point = matrix.apply(*point);
    }
}

fn hidden(item: &Value) -> bool {
    item.get("hd").and_then(Value::as_bool).unwrap_or(false)
}

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

/// Gradient fills are approximated by the average of their colour stops.
/// Geometry, depth and animation still come from the exact vector path.
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

/// Expand drawable paths into fixed topology slots.
/// Returns `(contour, closed, is_trimmed)` triplets. Untrimmed contours are
/// borrowed, so per-frame evaluation does not copy the point buffers.
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
                    let closed = item.contour.closed && *length >= 0.999;
                    slots.push((Cow::Owned(contour), closed, true));
                }
                // Keep a stable second slot even when it is empty.
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

pub fn is_animated(value: &Value) -> bool {
    value.get("a").and_then(Value::as_i64).unwrap_or(0) != 0
}

/// Whether shape geometry (paths, primitives, trim, width, group transforms)
/// changes over time. Colour-only animation is excluded.
pub fn geometry_animated(shapes: &[Value]) -> bool {
    for item in shapes {
        let ty = item.get("ty").and_then(Value::as_str).unwrap_or("");
        match ty {
            "sh" => {
                if item.get("ks").map(is_animated).unwrap_or(false) {
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

/// Whether a fill/stroke colour or opacity is keyframed.
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
