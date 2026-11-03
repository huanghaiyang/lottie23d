//! Lottie 文档的极简类型化模型，供转换器反序列化使用。
//!
//! 模块职责：只映射转换过程真正读取的字段，其余原始数据（`ks`、`shapes`）
//! 保留为 `serde_json::Value`，交给 `prop` / `shape` 按需动态解析。
//!
//! 单位约定：
//! * `fr` 为帧率（帧/秒），`ip` / `op` / `st` 为帧号，`sr` 为时间拉伸倍数；
//! * `w` / `h`、`solid_width` / `solid_height` 均为 Lottie 像素；
//! * 图层 `ty`：0 = 预合成，1 = 纯色层，4 = 形状层（本转换器只支持这三类）。

use serde::Deserialize;
use serde_json::Value;

/// 顶层文档：画布尺寸、时间范围以及顶层图层与资源。
#[derive(Deserialize, Clone, Debug)]
pub struct Lottie {
    /// 帧率（帧/秒），缺省 30。
    #[serde(default = "default_frame_rate")]
    pub fr: f32,
    /// 入点（帧）。
    #[serde(default)]
    pub ip: f32,
    /// 出点（帧），必须大于 `ip`。
    #[serde(default)]
    pub op: f32,
    /// 画布宽度（Lottie 像素），缺省 512。
    #[serde(default = "default_size")]
    pub w: f32,
    /// 画布高度（Lottie 像素），缺省 512。
    #[serde(default = "default_size")]
    pub h: f32,
    /// 动画名称。
    #[serde(default)]
    pub nm: String,
    /// 顶层图层列表，绘制顺序即数组顺序（后面的图层在上）。
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// 预合成 / 图片等资源。
    #[serde(default)]
    pub assets: Vec<Asset>,
}

/// 缺省帧率：30 fps。
fn default_frame_rate() -> f32 {
    30.0
}

/// 缺省画布宽/高：512 像素。
fn default_size() -> f32 {
    512.0
}

/// 单个图层。`ks` 为变换属性对象，`shapes` 为形状树原始节点。
#[derive(Deserialize, Clone, Debug)]
pub struct Layer {
    /// 图层编号，`parent` 通过它引用父图层。
    #[serde(default)]
    pub ind: i64,
    /// 图层类型：0 预合成、1 纯色、4 形状。
    #[serde(default)]
    pub ty: i64,
    /// 预合成图层引用的资源 id。
    #[serde(default, rename = "refId")]
    pub ref_id: String,
    /// 图层名称，顶点跟随表达式按名称查找空对象。
    #[serde(default)]
    pub nm: String,
    /// 变换属性（位置、锚点、缩放、旋转、透明度等），延迟解析。
    #[serde(default)]
    pub ks: Value,
    /// 形状树原始节点，延迟解析。
    #[serde(default)]
    pub shapes: Vec<Value>,
    /// 图层遮罩列表（原始节点，延迟解析）；遮罩路径与形状处于同一图层坐标系。
    #[serde(default, rename = "masksProperties")]
    pub masks: Vec<Value>,
    /// 图层效果列表（延迟解析）。用于读取 `Pseudo/ADBE Trace Path` 等
    /// 伪效果参数，复现「沿路径分布」的位置表达式。
    #[serde(default)]
    pub ef: Vec<Value>,
    /// 图层入点（父合成时间轴上的帧号）。
    #[serde(default)]
    pub ip: f32,
    /// 图层出点（父合成时间轴上的帧号）。
    #[serde(default)]
    pub op: f32,
    /// 起始时间偏移（帧），求值前需从合成帧中减去。
    #[serde(default)]
    pub st: f32,
    /// 时间拉伸倍数，`sr != 1` 时子合成时间需再除以它。
    #[serde(default = "default_stretch")]
    pub sr: f32,
    /// 时间重映射（AE Time Remap）：标量属性，值以秒计，仅预合成图层使用。
    /// 启用时优先于 `sr`，换算后成为子合成内部的时间轴。
    #[serde(default)]
    pub tm: Option<Value>,
    /// 父图层的 `ind`。
    #[serde(default)]
    pub parent: Option<i64>,
    /// 是否隐藏。
    #[serde(default)]
    pub hd: bool,
    /// 自动定向标志（本转换器不处理）。
    #[serde(default)]
    pub ao: i64,
    /// 是否 3D 图层。
    #[serde(default)]
    pub ddd: i64,
    /// 文本图层类型（本转换器忽略文本）。
    #[serde(default)]
    pub tt: i64,
    /// 纯色层颜色（`#rrggbb`）。
    #[serde(default, rename = "sc")]
    pub solid_color: String,
    /// 纯色层宽度（像素）。
    #[serde(default, rename = "sw")]
    pub solid_width: f32,
    /// 纯色层高度（像素）。
    #[serde(default, rename = "sh")]
    pub solid_height: f32,
}

/// 时间拉伸缺省值：1（不变速）。
fn default_stretch() -> f32 {
    1.0
}

/// 资源：预合成（图层列表）或图片（本转换器只用到预合成图层）。
#[derive(Deserialize, Clone, Debug)]
pub struct Asset {
    /// 资源 id，与图层的 `refId` 对应。
    #[serde(default)]
    pub id: String,
    /// 预合成内部的图层。
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// 资源宽度（像素）。
    #[serde(default)]
    pub w: f32,
    /// 资源高度（像素）。
    #[serde(default)]
    pub h: f32,
    /// 资源帧率（当前未使用）。
    #[serde(default)]
    pub fr: f32,
}

impl Lottie {
    /// 按 id 查找资源；找不到返回 `None`。
    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.id == id)
    }
}
