//! Minimal typed Lottie document model used by the converter.

use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize, Clone, Debug)]
pub struct Lottie {
    #[serde(default = "default_frame_rate")]
    pub fr: f32,
    #[serde(default)]
    pub ip: f32,
    #[serde(default)]
    pub op: f32,
    #[serde(default = "default_size")]
    pub w: f32,
    #[serde(default = "default_size")]
    pub h: f32,
    #[serde(default)]
    pub nm: String,
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

fn default_frame_rate() -> f32 {
    30.0
}

fn default_size() -> f32 {
    512.0
}

#[derive(Deserialize, Clone, Debug)]
pub struct Layer {
    #[serde(default)]
    pub ind: i64,
    #[serde(default)]
    pub ty: i64,
    #[serde(default, rename = "refId")]
    pub ref_id: String,
    #[serde(default)]
    pub nm: String,
    #[serde(default)]
    pub ks: Value,
    #[serde(default)]
    pub shapes: Vec<Value>,
    #[serde(default)]
    pub ip: f32,
    #[serde(default)]
    pub op: f32,
    #[serde(default)]
    pub st: f32,
    #[serde(default = "default_stretch")]
    pub sr: f32,
    #[serde(default)]
    pub parent: Option<i64>,
    #[serde(default)]
    pub hd: bool,
    #[serde(default)]
    pub ao: i64,
    #[serde(default)]
    pub ddd: i64,
    #[serde(default)]
    pub tt: i64,
    #[serde(default, rename = "sc")]
    pub solid_color: String,
    #[serde(default, rename = "sw")]
    pub solid_width: f32,
    #[serde(default, rename = "sh")]
    pub solid_height: f32,
}

fn default_stretch() -> f32 {
    1.0
}

#[derive(Deserialize, Clone, Debug)]
pub struct Asset {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub h: f32,
    #[serde(default)]
    pub fr: f32,
}

impl Lottie {
    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.id == id)
    }
}
