/**
 * lottie2mesh JavaScript SDK.
 *
 * Loads the Rust converter (WebAssembly) once and turns Lottie JSON into a
 * compact binary model. The same module works in Node.js and in the browser,
 * so other projects can import it without any build step:
 *
 * ```js
 * import { loadConverter } from 'lottie2mesh';
 * const converter = await loadConverter();
 * const model = converter.convert(lottieJson, { fps: 30 });
 * ```
 *
 * The returned model holds plain typed arrays that can be uploaded to WebGL
 * buffers (three.js) without another pass over JSON.
 */

const MAGIC = 0x4c323344;
const HEADER_SIZE = 96;
const RECORD_SIZE = 96;

const isNode =
  typeof process !== "undefined" && process.versions != null && process.versions.node != null;

async function readWasm(source) {
  if (source == null) {
    if (isNode) {
      const { readFile } = await import("node:fs/promises");
      const bytes = await readFile(new URL("../public/wasm/lottie2mesh.wasm", import.meta.url));
      return new Uint8Array(bytes);
    }
    const url = new URL("wasm/lottie2mesh.wasm", document.baseURI);
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`无法加载 wasm（HTTP ${response.status}）：${url}`);
    }
    return new Uint8Array(await response.arrayBuffer());
  }

  if (source instanceof WebAssembly.Module) {
    return source;
  }
  if (source instanceof ArrayBuffer) {
    return new WebAssembly.Module(new Uint8Array(source));
  }
  if (ArrayBuffer.isView(source)) {
    return new WebAssembly.Module(source);
  }
  if (typeof source === "string" || source instanceof URL) {
    const target = source instanceof URL ? source : String(source);
    if (isNode) {
      const { readFile } = await import("node:fs/promises");
      const bytes = await readFile(target);
      return new WebAssembly.Module(new Uint8Array(bytes));
    }
    const response = await fetch(target);
    if (!response.ok) {
      throw new Error(`无法加载 wasm（HTTP ${response.status}）：${target}`);
    }
    return new WebAssembly.Module(new Uint8Array(await response.arrayBuffer()));
  }
  throw new TypeError("loadConverter() 需要 wasm 路径、wasm 字节或 WebAssembly.Module");
}

function readString(memory, pointer, length) {
  if (!pointer || !length) {
    return "";
  }
  return new TextDecoder().decode(new Uint8Array(memory.buffer, pointer, length));
}

/**
 * Decode the binary model emitted by the Rust compiler.
 * All integer/float reads use little-endian, matching Rust's output.
 */
export function parseModel(bytes) {
  let buffer = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (buffer.byteOffset % 4 !== 0) {
    buffer = buffer.slice();
  }
  if (buffer.byteLength < HEADER_SIZE) {
    throw new Error("模型数据不完整");
  }
  const view = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  if (view.getUint32(0, true) !== MAGIC) {
    throw new Error("模型 magic 不匹配（不是 lottie2mesh 输出）");
  }
  const meta = {
    version: view.getUint32(4, true),
    width: view.getFloat32(8, true),
    height: view.getFloat32(12, true),
    frameRate: view.getFloat32(16, true),
    ip: view.getFloat32(20, true),
    op: view.getFloat32(24, true),
    sampleFps: view.getFloat32(28, true),
    frameCount: view.getUint32(32, true),
    meshCount: view.getUint32(36, true),
    transformCount: view.getUint32(40, true),
    totalVertices: view.getUint32(44, true),
    totalIndices: view.getUint32(48, true),
    totalSize: view.getUint32(52, true),
    conversionUs: view.getUint32(56, true),
  };
  meta.name = readString(buffer, view.getUint32(64, true), view.getUint32(68, true));
  meta.frameCount = Math.max(1, meta.frameCount);
  meta.duration = meta.frameCount / Math.max(1e-4, meta.sampleFps);

  const transformStart = HEADER_SIZE + RECORD_SIZE * meta.meshCount;
  const transformFloats = meta.frameCount * 16;
  const transforms = new Array(meta.transformCount);
  for (let index = 0; index < meta.transformCount; index += 1) {
    const offset = transformStart + index * transformFloats * 4;
    transforms[index] = new Float32Array(buffer.buffer, buffer.byteOffset + offset, transformFloats);
  }

  const meshes = new Array(meta.meshCount);
  for (let index = 0; index < meta.meshCount; index += 1) {
    const record = HEADER_SIZE + index * RECORD_SIZE;
    const kind = view.getUint32(record, true);
    const flags = view.getUint32(record + 4, true);
    const color = new Float32Array(4);
    for (let c = 0; c < 4; c += 1) {
      color[c] = view.getFloat32(record + 8 + c * 4, true);
    }
    const vertexCount = view.getUint32(record + 32, true);
    const indexCount = view.getUint32(record + 36, true);
    const positionsOffset = view.getUint32(record + 40, true);
    const framePositionsOffset = view.getUint32(record + 44, true);
    const indicesOffset = view.getUint32(record + 48, true);
    const colorTrackOffset = view.getUint32(record + 52, true);
    const pointCount = view.getUint32(record + 64, true);
    const name = readString(buffer, view.getUint32(record + 68, true), view.getUint32(record + 72, true));

    const positions = new Float32Array(
      buffer.buffer,
      buffer.byteOffset + positionsOffset,
      vertexCount * 3,
    );
    const indices = new Uint32Array(buffer.buffer, buffer.byteOffset + indicesOffset, indexCount);
    const colors =
      flags & 2
        ? new Float32Array(
            buffer.buffer,
            buffer.byteOffset + colorTrackOffset,
            meta.frameCount * 4,
          )
        : null;
    const framePositions =
      flags & 1
        ? new Float32Array(
            buffer.buffer,
            buffer.byteOffset + framePositionsOffset,
            meta.frameCount * vertexCount * 3,
          )
        : null;

    meshes[index] = {
      index,
      kind,
      animated: (flags & 1) === 1,
      color,
      depth: view.getFloat32(record + 24, true),
      halfThickness: view.getFloat32(record + 28, true),
      vertexCount,
      indexCount,
      pointCount,
      positions,
      framePositions,
      indices,
      colors,
      transformIndex: view.getUint32(record + 56, true),
      renderOrder: view.getUint32(record + 60, true),
      name,
    };
  }
  meshes.sort((a, b) => a.renderOrder - b.renderOrder);

  // Stable node list for UIs and external tools. Node ids follow the render
  // order, which is the same order three.js builds its meshes in.
  const nodes = meshes.map((mesh, id) => ({
    id,
    name: mesh.name,
    kind: mesh.kind,
    animated: mesh.animated,
    vertexCount: mesh.vertexCount,
    indexCount: mesh.indexCount,
    renderOrder: mesh.renderOrder,
  }));

  return { ...meta, transforms, meshes, nodes };
}

/** Instantiate the converter. Pass a URL/path/bytes to override the default wasm. */
export async function loadConverter(source) {
  const module = await readWasm(source);
  const result = await WebAssembly.instantiate(module, {});
  const instance = result instanceof WebAssembly.Instance ? result : result.instance;
  const exports = instance.exports;
  const memory = exports.memory;
  if (!memory) {
    throw new Error("wasm 实例没有导出 memory");
  }

  const encoder = new TextEncoder();

  /** Convert to the raw `.l2d3` binary buffer (no decoding). */
  function convertBytes(lottie, options = {}) {
    const json = typeof lottie === "string" ? lottie : JSON.stringify(lottie);
    const jsonBytes = encoder.encode(json);
    const optionBytes =
      Object.keys(options).length > 0 ? encoder.encode(JSON.stringify(options)) : null;

    const jsonPtr = exports.l2d_alloc(jsonBytes.length);
    if (!jsonPtr) {
      throw new Error("wasm 内存分配失败");
    }
    new Uint8Array(memory.buffer, jsonPtr, jsonBytes.length).set(jsonBytes);

    let optionPtr = 0;
    let optionLen = 0;
    if (optionBytes) {
      optionPtr = exports.l2d_alloc(optionBytes.length);
      new Uint8Array(memory.buffer, optionPtr, optionBytes.length).set(optionBytes);
      optionLen = optionBytes.length;
    }

    let status = 1;
    try {
      status = exports.l2d_convert(jsonPtr, jsonBytes.length, optionPtr, optionLen);
    } finally {
      exports.l2d_dealloc(jsonPtr, jsonBytes.length);
      if (optionPtr) {
        exports.l2d_dealloc(optionPtr, optionLen);
      }
    }

    if (status !== 0) {
      const message =
        readString(memory, exports.l2d_error_ptr(), exports.l2d_error_len()) || "转换失败";
      exports.l2d_release();
      throw new Error(message);
    }

    const outputPtr = exports.l2d_output_ptr();
    const outputLen = exports.l2d_output_len();
    const bytes = new Uint8Array(memory.buffer, outputPtr, outputLen).slice();
    exports.l2d_release();
    return bytes;
  }

  function convert(lottie, options = {}) {
    return parseModel(convertBytes(lottie, options));
  }

  return {
    convert,
    convertBytes,
    parseModel,
    version: exports.l2d_version(),
  };
}

export default loadConverter;
