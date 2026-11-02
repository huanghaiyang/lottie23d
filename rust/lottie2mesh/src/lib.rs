//! lottie2mesh — compile Lottie vector animations into extruded 3D meshes.
//!
//! The crate exposes a small C ABI so the same binary can be loaded from
//! Node.js (WebAssembly), from a browser, or linked into another Rust project.
//!
//! Flow (JS / Node):
//!
//! ```text
//! const bytes = l2d_alloc(json.length)        // write JSON into memory
//! l2d_convert(bytes, len, optionsPtr, optLen) // 0 = success
//! const model = new Uint8Array(l2d_output_ptr(), l2d_output_len())
//! l2d_release()
//! ```
//!
//! Everything is deterministic and allocation-free per animation frame: the
//! expensive geometry work happens once during conversion.

pub mod compile;
pub mod lottie;
pub mod mesh;
pub mod output;
pub mod path;
pub mod prop;
pub mod shape;

use std::sync::{Mutex, OnceLock};

use compile::RawOptions;

/// Direct Rust API shared by the CLI and other Rust projects.
pub fn compile_to_bytes(json: &str, options: RawOptions) -> Result<Vec<u8>, String> {
    let model = compile::compile(json, options)?;
    Ok(output::encode(&model))
}

/* ------------------------------------------------------------------ */
/* C ABI state                                                         */
/* ------------------------------------------------------------------ */

struct State {
    output: Vec<u8>,
    error: String,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| {
        Mutex::new(State {
            output: Vec::new(),
            error: String::new(),
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    match state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/* ------------------------------------------------------------------ */
/* Memory                                                              */
/* ------------------------------------------------------------------ */

/// Allocate `len` zeroed bytes and hand ownership to the caller.
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

/// Release a buffer previously returned by [`l2d_alloc`].
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

/// Convert a UTF-8 Lottie JSON document.
///
/// `options_ptr` may be null; when present it is a UTF-8 JSON object with
/// optional `fps`, `max_frames` and `thickness` fields.
///
/// Returns `0` on success, `1` on failure (see [`l2d_error_ptr`]).
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

fn set_error(message: String) {
    let mut guard = lock();
    guard.error = message;
    guard.output.clear();
}

/// Pointer to the last successful conversion result.
#[no_mangle]
pub extern "C" fn l2d_output_ptr() -> *const u8 {
    let guard = lock();
    if guard.output.is_empty() {
        std::ptr::null()
    } else {
        guard.output.as_ptr()
    }
}

/// Length in bytes of the last successful conversion result.
#[no_mangle]
pub extern "C" fn l2d_output_len() -> u32 {
    let guard = lock();
    guard.output.len() as u32
}

/// Drop the retained output buffer.
#[no_mangle]
pub extern "C" fn l2d_release() {
    let mut guard = lock();
    guard.output = Vec::new();
    guard.error.clear();
}

/// Pointer to the last error message (UTF-8), or null when none.
#[no_mangle]
pub extern "C" fn l2d_error_ptr() -> *const u8 {
    let guard = lock();
    if guard.error.is_empty() {
        std::ptr::null()
    } else {
        guard.error.as_ptr()
    }
}

/// Length in bytes of the last error message.
#[no_mangle]
pub extern "C" fn l2d_error_len() -> u32 {
    let guard = lock();
    guard.error.len() as u32
}

/// ABI version, bumped when the exported surface changes.
#[no_mangle]
pub extern "C" fn l2d_version() -> u32 {
    1
}
