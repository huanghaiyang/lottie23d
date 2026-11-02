#!/usr/bin/env node
// Convert a Lottie file into the compact `.l2d3` binary model.
//
//   node sdk/cli.mjs input.json [output.l2d3] [--fps=30] [--max-frames=600]

import { readFile, writeFile } from "node:fs/promises";
import { loadConverter, parseModel } from "./index.mjs";

const args = process.argv.slice(2);
let input = null;
let output = null;
const options = {};

for (const arg of args) {
  if (arg === "-h" || arg === "--help") {
    console.log(
      "用法: node sdk/cli.mjs <input.json> [output.l2d3] [--fps=30] [--max-frames=600] [--thickness=10]",
    );
    process.exit(0);
  }
  const match = /^--([a-z-]+)=(.*)$/.exec(arg);
  if (match) {
    const [, key, value] = match;
    if (key === "fps") options.fps = Number(value);
    else if (key === "max-frames") options.max_frames = Number(value);
    else if (key === "thickness") options.thickness = Number(value);
    continue;
  }
  if (input == null) input = arg;
  else if (output == null) output = arg;
}

if (!input) {
  console.error("缺少输入文件。使用 --help 查看用法。");
  process.exit(1);
}

const json = await readFile(input, "utf8");
const converter = await loadConverter();
const started = performance.now();
const bytes = converter.convertBytes(json, options);
const elapsed = performance.now() - started;
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
if (!output) {
  process.stdout.write(bytes);
}
