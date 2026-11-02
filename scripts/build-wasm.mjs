#!/usr/bin/env node
// Build the Rust converter for wasm32 and publish it to public/wasm.
// Run: npm run build:wasm

import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const manifest = join(root, "rust", "lottie2mesh", "Cargo.toml");
const artifact = join(
  root,
  "rust",
  "lottie2mesh",
  "target",
  "wasm32-unknown-unknown",
  "release",
  "lottie2mesh.wasm",
);
const destination = join(root, "public", "wasm", "lottie2mesh.wasm");

function findCargo() {
  if (process.env.CARGO) {
    return process.env.CARGO;
  }
  const local = join(homedir(), ".cargo", "bin", process.platform === "win32" ? "cargo.exe" : "cargo");
  const probe = spawnSync(local, ["--version"], { encoding: "utf8" });
  if (probe.status === 0) {
    return local;
  }
  return process.platform === "win32" ? "cargo.exe" : "cargo";
}

const cargo = findCargo();
const result = spawnSync(
  cargo,
  ["build", "--release", "--lib", "--target", "wasm32-unknown-unknown", "--manifest-path", manifest],
  { stdio: "inherit" },
);
if (result.status !== 0) {
  console.error("wasm 构建失败");
  process.exit(result.status ?? 1);
}

mkdirSync(join(root, "public", "wasm"), { recursive: true });
copyFileSync(artifact, destination);
const size = (statSync(destination).size / 1024).toFixed(1);
console.log(`wasm 已输出到 public/wasm/lottie2mesh.wasm（${size} KB）`);
