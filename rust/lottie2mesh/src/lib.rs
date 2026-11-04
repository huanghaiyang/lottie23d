//! lottie2mesh —— 把 Lottie 矢量动画编译成可挤出的 3D 网格。
//!
//! 模块职责：
//! * 对外提供两条入口：Rust 侧的 [`compile_to_bytes`] 与 C ABI（`l2d_*`），
//!   同一份 wasm 二进制可被 Node.js、浏览器加载，也可被其他 Rust 项目链接；
//! * 维护 C ABI 的全局输出缓冲区与错误信息（单例 `Mutex`），负责跨 FFI
//!   边界的分配/释放。
//!
//! 关键数据流：
//! ```text
//! Lottie JSON 字符串
//!   → compile::compile  (逐层求值 + 三角化 + 挤出 + 逐帧烘焙)
//!   → Compiled 模型
//!   → output::encode    (序列化为 v2 二进制)
//!   → Vec<u8> / wasm 内存
//! ```
//!
//! 单位约定：
//! * 所有几何、厚度（`thickness` / `half_thickness`）都在 Lottie 像素空间，
//!   只有最终写出的全局矩阵（`3.6 / max(w, h)` 缩放）才把它映射到屏幕空间；
//! * 输出二进制中的坐标、矩阵、颜色分量均为 f32，具体列主序见 `output.rs`；
//! * 时间单位为「帧」，帧率由 `fr` / 采样 `fps` 决定。
//!
//! JS / Node 调用流程：
//!
//! ```text
//! const bytes = l2d_alloc(json.length)        // 把 JSON 写入 wasm 内存
//! l2d_convert(bytes, len, optionsPtr, optLen) // 0 表示成功
//! const model = new Uint8Array(l2d_output_ptr(), l2d_output_len())
//! l2d_release()
//! ```
//!
//! 整个转换是确定性的：昂贵的几何计算只在转换期做一次，播放期无需再分配。

pub mod boolops;
pub mod compile;
pub mod lottie;
pub mod mesh;
pub mod output;
pub mod path;
pub mod prop;
pub mod shape;

use std::sync::{Mutex, OnceLock};

use compile::RawOptions;

/// Rust 直调入口：解析 JSON、编译模型并编码为二进制。
///
/// * `json`：Lottie 文档原文（UTF-8）。
/// * `options`：采样帧率、最大帧数与挤出参数；缺省值由 `compile` 内部决定。
///
/// 返回 v2 二进制模型（布局见 `output.rs`）；失败时返回中文错误信息。
pub fn compile_to_bytes(json: &str, options: RawOptions) -> Result<Vec<u8>, String> {
    let model = compile::compile(json, options)?;
    Ok(output::encode(&model))
}

/* ------------------------------------------------------------------ */
/* C ABI state                                                         */
/* ------------------------------------------------------------------ */

/// C ABI 的全局状态：最近一次成功的输出缓冲与最近一次错误文本。
struct State {
    output: Vec<u8>,
    error: String,
}

/// 进程内唯一的状态单例；`OnceLock` 保证 wasm 多实例下也只初始化一次。
static STATE: OnceLock<Mutex<State>> = OnceLock::new();

/// 取得（必要时惰性初始化）全局状态互斥锁。
fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            output: Vec::new(),
            error: String::new(),
        })
    })
}

/// 加锁；若前一次持锁线程 panic（毒化锁），直接取出内部状态继续使用。
fn lock() -> std::sync::MutexGuard<'static, State> {
    match state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/* ------------------------------------------------------------------ */
/* Memory                                                              */
/* ------------------------------------------------------------------ */

/// 分配 `len` 字节的零初始化内存，并把所有权移交给调用方。
///
/// 返回的指针需通过 [`l2d_dealloc`] 释放；`len == 0` 时返回空指针。
/// `Vec` 被 `mem::forget` 后不再由 Rust 管理，长度信息必须由调用方保存。
#[no_mangle]
pub extern "C" fn l2d_alloc(len: u32) -> *mut u8 {
    let size = len as usize;
    if size == 0 {
        return std::ptr::null_mut();
    }
    let mut buffer: Vec<u8> = vec![0u8; size];
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// 释放此前由 [`l2d_alloc`] 返回的缓冲区。
///
/// `len` 必须与分配时一致，否则重建 `Vec` 的容量信息会出错；
/// 空指针或长度为 0 时直接返回。
#[no_mangle]
pub unsafe extern "C" fn l2d_dealloc(pointer: *mut u8, len: u32) {
    let size = len as usize;
    if pointer.is_null() || size == 0 {
        return;
    }
    drop(Vec::from_raw_parts(pointer, size, size));
}

/* ------------------------------------------------------------------ */
/* Conversion                                                          */
/* ------------------------------------------------------------------ */

/// 转换一段 UTF-8 的 Lottie JSON。
///
/// * `json_ptr` / `json_len`：输入 JSON 的起始地址与字节数，不能为空。
/// * `options_ptr` / `options_len`：可选的 JSON 选项对象，可为空；字段包括
///   `fps`、`max_frames`、`thickness`、`edge_radius`（别名 `edgeRadius`）。
///
/// 成功返回 `0`，此时结果可用 [`l2d_output_ptr`] / [`l2d_output_len`] 读取；
/// 失败返回 `1`，错误文本可用 [`l2d_error_ptr`] / [`l2d_error_len`] 读取。
#[no_mangle]
pub unsafe extern "C" fn l2d_convert(
    json_ptr: *const u8,
    json_len: u32,
    options_ptr: *const u8,
    options_len: u32,
) -> u32 {
    if json_ptr.is_null() || json_len == 0 {
        set_error("输入为空".to_string());
        return 1;
    }
    let json_bytes = std::slice::from_raw_parts(json_ptr, json_len as usize);
    let json = match std::str::from_utf8(json_bytes) {
        Ok(text) => text,
        Err(error) => {
            set_error(format!("输入不是合法 UTF-8: {error}"));
            return 1;
        }
    };

    let options = match parse_options(options_ptr, options_len) {
        Ok(options) => options,
        Err(error) => {
            set_error(error);
            return 1;
        }
    };

    match compile_to_bytes(json, options) {
        Ok(bytes) => {
            let mut guard = lock();
            guard.output = bytes;
            guard.error.clear();
            0
        }
        Err(error) => {
            set_error(error);
            1
        }
    }
}

/// 解析可选的 C ABI 选项 JSON；指针为空、长度为 0 或内容为空白时用默认值。
unsafe fn parse_options(pointer: *const u8, len: u32) -> Result<RawOptions, String> {
    if pointer.is_null() || len == 0 {
        return Ok(RawOptions::default());
    }
    let bytes = std::slice::from_raw_parts(pointer, len as usize);
    let text = std::str::from_utf8(bytes).map_err(|error| format!("选项不是合法 UTF-8: {error}"))?;
    if text.trim().is_empty() {
        return Ok(RawOptions::default());
    }
    serde_json::from_str::<RawOptions>(text).map_err(|error| format!("选项解析失败: {error}"))
}

/// 记录错误并清空上一次的输出，避免调用方读到过期结果。
fn set_error(message: String) {
    let mut guard = lock();
    guard.error = message;
    guard.output.clear();
}

/// 最近一次成功转换结果的起始指针；没有结果时返回空指针。
///
/// 指针在下一次 [`l2d_release`] 或再次转换前有效。
#[no_mangle]
pub extern "C" fn l2d_output_ptr() -> *const u8 {
    let guard = lock();
    if guard.output.is_empty() {
        std::ptr::null()
    } else {
        guard.output.as_ptr()
    }
}

/// 最近一次成功转换结果的字节长度。
#[no_mangle]
pub extern "C" fn l2d_output_len() -> u32 {
    let guard = lock();
    guard.output.len() as u32
}

/// 释放保留的输出缓冲与错误文本（不涉及调用方自行分配的内存）。
#[no_mangle]
pub extern "C" fn l2d_release() {
    let mut guard = lock();
    guard.output = Vec::new();
    guard.error.clear();
}

/// 最近一次错误文本（UTF-8）的起始指针；没有错误时返回空指针。
#[no_mangle]
pub extern "C" fn l2d_error_ptr() -> *const u8 {
    let guard = lock();
    if guard.error.is_empty() {
        std::ptr::null()
    } else {
        guard.error.as_ptr()
    }
}

/// 最近一次错误文本的字节长度。
#[no_mangle]
pub extern "C" fn l2d_error_len() -> u32 {
    let guard = lock();
    guard.error.len() as u32
}

/// ABI 版本号，导出接口变化时需要递增；当前 4 与二进制格式版本一致。
#[no_mangle]
pub extern "C" fn l2d_version() -> u32 {
    4
}
