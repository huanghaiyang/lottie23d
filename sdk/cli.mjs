#!/usr/bin/env node
// 命令行入口：把一个 Lottie JSON 转换成紧凑的 `.l2d3` 二进制模型。
//
// 数据流：读取 JSON → loadConverter() 加载 wasm → convertBytes() 得到二进制
//         → parseModel() 解析出统计信息 → 写文件或直接输出到 stdout。
//
//   node sdk/cli.mjs input.json [output.l2d3] [--fps=30] [--max-frames=600]

import { readFile, writeFile } from "node:fs/promises";
import { loadConverter, parseModel } from "./index.mjs";

// 参数约定：第一个位置参数是输入 JSON，第二个可选位置参数是输出 `.l2d3`；
// `--key=value` 形式转发为转换选项，与 Rust 侧的字段名保持一致。
const args = process.argv.slice(2);
let input = null;
let output = null;
const options = {};

for (const arg of args) {
  if (arg === "-h" || arg === "--help") {
    console.log(
      "用法: node sdk/cli.mjs <input.json> [output.l2d3] [--fps=30] [--max-frames=600] [--thickness=10] [--edge-radius=4] [--curve-spacing=3]",
    );
    process.exit(0);
  }
  const match = /^--([a-z-]+)=(.*)$/.exec(arg);
  if (match) {
    const [, key, value] = match;
    if (key === "fps") options.fps = Number(value);
    else if (key === "max-frames") options.max_frames = Number(value);
    else if (key === "thickness") options.thickness = Number(value);
    else if (key === "edge-radius") options.edge_radius = Number(value);
    else if (key === "curve-spacing") options.curve_spacing = Number(value);
    continue;
  }
  if (input == null) input = arg;
  else if (output == null) output = arg;
}

if (!input) {
  console.error("缺少输入文件。使用 --help 查看用法。");
  process.exit(1);
}

// 直接按 UTF-8 读取文本，避免 JSON.parse → stringify 的一次多余往返。
const json = await readFile(input, "utf8");
const converter = await loadConverter();
// 只统计转换本身（不含 wasm 加载），便于对比不同参数的开销。
const started = performance.now();
const bytes = converter.convertBytes(json, options);
const elapsed = performance.now() - started;
// 解析仅用于打印统计；输出文件仍写入原始字节，避免重新序列化。
const model = parseModel(bytes);

if (output) {
  await writeFile(output, bytes);
}

const kb = (bytes.byteLength / 1024).toFixed(1);
console.log(
  `${model.name || "未命名"} | ${model.width}×${model.height} @ ${model.sampleFps.toFixed(1)}fps | ` +
    `${model.frameCount} 帧 | ${model.meshCount} 网格 | ${model.totalVertices} 顶点 | ` +
    `${model.totalIndices / 3} 三角形 | ${kb} KB | ${elapsed.toFixed(1)} ms`,
);
// 没指定输出文件时把二进制写到 stdout，便于管道重定向（`> out.l2d3`）。
if (!output) {
  process.stdout.write(bytes);
}
