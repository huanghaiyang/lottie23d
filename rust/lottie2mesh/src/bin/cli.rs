//! 命令行入口：`lottie2mesh-cli <input.json> [output.l2d3]`。
//!
//! 参数：
//! * `--fps=<n>` 采样帧率；
//! * `--max-frames=<n>` 烘焙帧数上限；
//! * `--thickness=<px>` 挤出半厚（Lottie 像素）；
//! * `--edge-radius=<px>` 边缘圆角半径（Lottie 像素）；
//! * 输入为 `-` 时从 stdin 读取 JSON；省略输出路径时二进制写到 stdout。
//!
//! 统计摘要始终输出到 stderr，避免污染 stdout 上的二进制数据。

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

use lottie2mesh::compile::{RawOptions, compile};

/// 解析参数、读取 JSON、调用 [`compile`] 并写出二进制；返回进程退出码。
fn main() -> ExitCode {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut options = RawOptions::default();

    // 选项只接受 `--key=value` 形式；按出现顺序，第一个非选项参数为输入，
    // 第二个为输出路径。
    for arg in env::args().skip(1) {
        if arg == "-h" || arg == "--help" {
            print_help();
            return ExitCode::SUCCESS;
        }
        if let Some(value) = arg.strip_prefix("--fps=") {
            options.fps = value.parse::<f32>().ok();
            continue;
        }
        if let Some(value) = arg.strip_prefix("--max-frames=") {
            options.max_frames = value.parse::<usize>().ok();
            continue;
        }
        if let Some(value) = arg.strip_prefix("--thickness=") {
            options.thickness = value.parse::<f32>().ok();
            continue;
        }
        if let Some(value) = arg.strip_prefix("--edge-radius=") {
            options.edge_radius = value.parse::<f32>().ok();
            continue;
        }
        if input.is_none() {
            input = Some(arg);
        } else if output.is_none() {
            output = Some(arg);
        }
    }

    let Some(input) = input else {
        print_help();
        return ExitCode::FAILURE;
    };

    let json = match input.as_str() {
        // 输入为 `-` 时从标准输入读取 JSON。
        "-" => read_stdin(),
        _ => fs::read_to_string(&input)
            .map_err(|error| format!("无法读取 {input}: {error}")),
    };
    let json = match json {
        Ok(json) => json,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let model = match compile(&json, options) {
        Ok(model) => model,
        Err(error) => {
            eprintln!("转换失败: {error}");
            return ExitCode::FAILURE;
        }
    };

    let bytes = lottie2mesh::output::encode(&model);
    match output {
        Some(path) => {
            if let Err(error) = fs::write(&path, &bytes) {
                eprintln!("无法写入 {path}: {error}");
                return ExitCode::FAILURE;
            }
        }
        None => {
            // 省略输出路径时把二进制直接写到 stdout，便于 shell 管道。
            let _ = io::stdout().write_all(&bytes);
        }
    }

    // 统计摘要写到 stderr，stdout 只保留二进制数据。
    let vertices: usize = model
        .meshes
        .iter()
        .map(|mesh| mesh.positions.len() / 3)
        .sum();
    let triangles: usize = model
        .meshes
        .iter()
        .map(|mesh| mesh.indices.len() / 3)
        .sum();

    eprintln!(
        "{} | {}×{} @ {}fps | {} 帧 | {} 网格 | {} 顶点 | {} 三角形 | {:.1} KB | 转换 {:.1} ms",
        model.name,
        model.width,
        model.height,
        model.sample_fps,
        model.frame_count,
        model.meshes.len(),
        vertices,
        triangles,
        bytes.len() as f32 / 1024.0,
        model.conversion_us as f32 / 1000.0,
    );
    ExitCode::SUCCESS
}

/// 读取标准输入的全部文本；失败时返回中文错误信息。
fn read_stdin() -> Result<String, String> {
    let mut buffer = String::new();
    io::stdin()
        .read_to_string(&mut buffer)
        .map_err(|error| format!("无法读取标准输入: {error}"))?;
    Ok(buffer)
}

/// 把用法说明打印到 stderr。
fn print_help() {
    eprintln!(
        "lottie2mesh-cli <input.json> [output.l2d3]\n\
          \n\
          Options:\n\
          \x20 --fps=<n>          sampling fps (default: document frame rate)\n\
          \x20 --max-frames=<n>   cap on baked frames (default 600)\n\
          \x20 --thickness=<px>   extrusion half-thickness in Lottie pixels\n\
          \x20 --edge-radius=<px> edge smoothing radius in Lottie pixels\n\
          \x20                     (clamped to the half-thickness, default 0)\n\
          \n\
          Pass `-` as input to read JSON from stdin; omit the output path to\n\
          stream the binary model to stdout."
    );
}
