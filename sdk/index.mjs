/**
 * lottie2mesh JavaScript SDK（无构建依赖，Node.js 与浏览器共用同一份 ESM）。
 *
 * 职责与数据流：
 * 1. 加载 Rust 转换器编译出的 WebAssembly 模块（默认取 public/wasm/lottie2mesh.wasm）；
 * 2. 通过 C ABI 把 Lottie JSON 与选项交给 wasm，得到 `.l2d3` 二进制字节；
 * 3. parseModel() 把二进制零拷贝解析成 TypedArray 视图（顶点、索引、逐帧矩阵等）；
 * 4. 上层（如 src/scene/stage.ts）可直接把结果上传到 three.js 的 WebGL 缓冲，
 *    播放阶段无需再次解析 JSON 或做三角化。
 *
 * ```js
 * import { loadConverter } from 'lottie2mesh';
 * const converter = await loadConverter();
 * const model = converter.convert(lottieJson, { fps: 30 });
 * ```
 */

/** magic：`0x4c323344`，即 ASCII "L23D"，用于识别 lottie2mesh 的输出。 */
const MAGIC = 0x4c323344;
/** 文件头固定 96 字节（字段偏移见 parseModel 的注释表）。 */
const HEADER_SIZE = 96;
/** 每条 mesh record 固定 96 字节。 */
const RECORD_SIZE = 96;

/** 运行环境判定：Node 下存在 process.versions.node，浏览器下没有。 */
const isNode =
  typeof process !== "undefined" && process.versions != null && process.versions.node != null;

/**
 * 把调用方传入的 wasm 描述统一解析为 `WebAssembly.Module`。
 *
 * 支持的分支：undefined（默认路径）、WebAssembly.Module、ArrayBuffer、
 * TypedArray/DataView 视图、字符串路径、URL。Node 读本地文件，浏览器走 fetch，
 * 两者最终都编译成可复用的 Module，避免同一份字节被重复编译。
 *
 * @param source 可选；省略时使用默认 wasm 位置
 * @returns 已编译的 wasm 模块
 */
async function readWasm(source) {
  if (source == null) {
    // 未显式指定 wasm：Node 直接读仓库里的产物（相对 sdk/ 目录取 ../public）。
    if (isNode) {
      const { readFile } = await import("node:fs/promises");
      const bytes = await readFile(new URL("../public/wasm/lottie2mesh.wasm", import.meta.url));
      return new Uint8Array(bytes);
    }
    // 浏览器：public/ 会被原样发布，因此按站点基准地址取 wasm/ 下的文件。
    const url = new URL("wasm/lottie2mesh.wasm", document.baseURI);
    const response = await fetch(url);
    if (!response.ok) {
      throw new Error(`无法加载 wasm（HTTP ${response.status}）：${url}`);
    }
    return new Uint8Array(await response.arrayBuffer());
  }

  // 已经是编译好的模块：直接复用，省去一次编译。
  if (source instanceof WebAssembly.Module) {
    return source;
  }
  // 原始字节：包一层 Uint8Array 只是视图，不复制数据。
  if (source instanceof ArrayBuffer) {
    return new WebAssembly.Module(new Uint8Array(source));
  }
  // TypedArray / DataView：本身即 BufferSource，可交给 Module 构造器。
  if (ArrayBuffer.isView(source)) {
    return new WebAssembly.Module(source);
  }
  // 路径或 URL：Node 走 fs，浏览器走 fetch，得到字节后再编译。
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

/**
 * 从 wasm 线性内存中读取一段 UTF-8 字符串（名称、错误信息等）。
 * 指针或长度为 0 时视为空串，避免越界访问。
 *
 * @param memory wasm 导出的线性内存
 * @param pointer 起始字节偏移
 * @param length 字节长度（不含补零）
 * @returns 解码后的字符串
 */
function readString(memory, pointer, length) {
  if (!pointer || !length) {
    return "";
  }
  return new TextDecoder().decode(new Uint8Array(memory.buffer, pointer, length));
}

/**
 * 解析 Rust 编译器输出的 `.l2d3` 二进制模型。
 *
 * 所有整数 / 浮点均按小端读取，与 output.rs 的写入顺序一致；偏移都是相对
 * blob 起始的绝对字节偏移。返回的 TypedArray 多数是直接建立在同一块字节缓冲上的
 * **零拷贝视图**，因此不要再修改入参 bytes，也不要假设它们拥有独立内存。
 *
 * 文件头（HEADER_SIZE = 96 字节）字段偏移：
 *   +0  u32 magic          `0x4c323344`（"L23D"）
 *   +4  u32 version        格式版本，当前 v2（新增拓扑分桶）
 *   +8  f32 width          画布宽（Lottie px）
 *   +12 f32 height         画布高（Lottie px）
 *   +16 f32 frameRate      文档帧率
 *   +20 f32 ip             源动画首帧
 *   +24 f32 op             源动画尾帧（不含）
 *   +28 f32 sampleFps      实际烘焙采样帧率
 *   +32 u32 frameCount     烘焙帧数
 *   +36 u32 meshCount      网格数
 *   +40 u32 transformCount 变换轨道数
 *   +44 u32 totalVertices  总顶点数（所有网格静态顶点之和）
 *   +48 u32 totalIndices   总索引数（所有拓扑桶之和）
 *   +52 u32 totalSize      文件总字节数
 *   +56 u32 conversionUs   原生转换耗时（微秒；wasm 为 0）
 *   +64 u32 namePtr        UTF-8 文档名在 blob 内的偏移
 *   +68 u32 nameLen        文档名字节长度
 *
 * @param bytes `.l2d3` 字节（Uint8Array 或其他 ArrayBuffer 视图）
 * @returns 含 header 元信息、transforms、meshes、nodes 的模型对象
 */
export function parseModel(bytes) {
  // 统一成 Uint8Array；若起点不是 4 字节对齐，则复制一份，
  // 保证后续 DataView / TypedArray 能安全建立（否则构造器会抛 RangeError）。
  let buffer = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  if (buffer.byteOffset % 4 !== 0) {
    buffer = buffer.slice();
  }
  // 连文件头都放不下，说明数据被截断。
  if (buffer.byteLength < HEADER_SIZE) {
    throw new Error("模型数据不完整");
  }
  const view = new DataView(buffer.buffer, buffer.byteOffset, buffer.byteLength);
  // magic 校验：防止把其它二进制误当成模型解析。
  if (view.getUint32(0, true) !== MAGIC) {
    throw new Error("模型 magic 不匹配（不是 lottie2mesh 输出）");
  }
  // header 元信息（字段含义见上方偏移表）。
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
  // 至少保留 1 帧，避免下游按 frameCount 做除法或切片时出现 0 长度。
  meta.frameCount = Math.max(1, meta.frameCount);
  // 时长（秒）= 帧数 / 采样帧率；sampleFps 兜底一个极小数防止除零。
  meta.duration = meta.frameCount / Math.max(1e-4, meta.sampleFps);

  // 变换轨道区紧跟在 mesh records 之后，每条轨道占 frameCount × 16 个 float
  // （每帧一个列主序 4×4 世界矩阵，可直接交给 THREE.Matrix4.fromArray）。
  // 这里只按字节偏移切片，不做拷贝。
  const transformStart = HEADER_SIZE + RECORD_SIZE * meta.meshCount;
  const transformFloats = meta.frameCount * 16;
  const transforms = new Array(meta.transformCount);
  for (let index = 0; index < meta.transformCount; index += 1) {
    const offset = transformStart + index * transformFloats * 4;
    transforms[index] = new Float32Array(buffer.buffer, buffer.byteOffset + offset, transformFloats);
  }

  // mesh record 区从 HEADER_SIZE 开始，每条 96 字节，字段偏移如下：
  //   +0  u32 kind                0 = 填充（fill），1 = 描边（stroke）
  //   +4  u32 flags               bit0 逐帧顶点 / bit1 逐帧颜色 / bit2 frameOffsets / bit3 拓扑分桶
  //   +8  f32 color[4]            参考帧 RGBA（sRGB，0..1）
  //   +24 f32 depth               归一化层深（-0.65..0.65），已烘焙进矩阵的 Z 平移
  //   +28 f32 halfThickness       挤出半厚度（Lottie px）
  //   +32 u32 vertexCount         跨所有拓扑桶的最大顶点数
  //   +36 u32 indexCount          桶 0 的索引数（兼容单拓扑调用方）
  //   +40 u32 positionsOffset     静态顶点 xyz 起始（字节）
  //   +44 u32 framePositionsOffset 逐帧顶点起始（字节）
  //   +48 u32 indicesOffset       桶 0 索引起始（字节）
  //   +52 u32 colorTrackOffset    逐帧颜色起始（字节）
  //   +56 u32 transformIndex      引用的变换轨道下标
  //   +60 u32 renderOrder         three.js 绘制顺序
  //   +64 u32 pointCount          参考轮廓点数（未乘挤出剖面 stride）
  //   +68 u32 namePtr / +72 u32 nameLen  UTF-8 网格名
  //   +76 u32 bucketCount         拓扑桶数量（单桶时为 1）
  //   +80 u32 bucketTableOffset   桶表起始（每项 16B）
  //   +84 u32 frameOffsetsOffset  逐帧顶点起点表（frameCount + 1 个 u32，单位为 float）
  //   +88 u32 frameBucketOffset   逐帧桶下标表（frameCount 个 u8）
  const meshes = new Array(meta.meshCount);
  for (let index = 0; index < meta.meshCount; index += 1) {
    const record = HEADER_SIZE + index * RECORD_SIZE;
    const kind = view.getUint32(record, true);
    const flags = view.getUint32(record + 4, true);
    // 静态颜色：4 个 f32（RGBA），不是 0..255。
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
    const bucketCount = view.getUint32(record + 76, true);
    const bucketTableOffset = view.getUint32(record + 80, true);
    const frameOffsetsOffset = view.getUint32(record + 84, true);
    const frameBucketOffset = view.getUint32(record + 88, true);

    // 参考帧顶点：xyz 交错，共 vertexCount × 3 个 float。
    const positions = new Float32Array(
      buffer.buffer,
      buffer.byteOffset + positionsOffset,
      vertexCount * 3,
    );
    // 桶 0 的索引：每 3 个 u32 构成一个三角形。
    const indices = new Uint32Array(buffer.buffer, buffer.byteOffset + indicesOffset, indexCount);
    // flags bit1（2）：存在逐帧颜色轨道，长度 frameCount × 4（RGBA）。
    const colors =
      flags & 2
        ? new Float32Array(
            buffer.buffer,
            buffer.byteOffset + colorTrackOffset,
            meta.frameCount * 4,
          )
        : null;

    // flags bit2（4）：存在逐帧顶点起点表。
    // v2 把每帧顶点连续拼接，不同拓扑桶的帧可以有不同顶点数，
    // 因此必须靠 frameOffsets 标记每帧的起点与终点（共 frameCount + 1 项）。
    const frameOffsets =
      flags & 4
        ? new Uint32Array(
            buffer.buffer,
            buffer.byteOffset + frameOffsetsOffset,
            meta.frameCount + 1,
          )
        : null;
    // flags bit0（1）：存在逐帧顶点。
    // 长度必须取 frameOffsets[frameCount] —— 表尾哨兵记录的是**总 float 个数**，
    // 不能写成 frameCount * vertexCount * 3（那是旧版等长布局的假设，v2 会越界/错位）。
    const framePositions =
      flags & 1
        ? new Float32Array(
            buffer.buffer,
            buffer.byteOffset + framePositionsOffset,
            frameOffsets ? frameOffsets[meta.frameCount] : meta.frameCount * vertexCount * 3,
          )
        : null;

    // 拓扑分桶：每种采样轮廓布局（轮廓数 + 量化密度 + 闭合性）缓存一套索引。
    // 播放时只切换 index，顶点缓冲不变，因此各桶顶点数可以不同。
    // flags bit3（8）：使用多桶；桶表项 16B 布局：
    //   +0 u32 indicesOffset、+4 u32 indexCount、+8 u32 vertexCount、+12 u32 pointCount。
    let buckets;
    if (flags & 8 && bucketCount > 0) {
      buckets = new Array(bucketCount);
      for (let b = 0; b < bucketCount; b += 1) {
        const entry = bucketTableOffset + b * 16;
        const bucketIndicesOffset = view.getUint32(entry, true);
        const bucketIndexCount = view.getUint32(entry + 4, true);
        const bucketVertexCount = view.getUint32(entry + 8, true);
        const bucketPointCount = view.getUint32(entry + 12, true);
        buckets[b] = {
          indices: new Uint32Array(
            buffer.buffer,
            buffer.byteOffset + bucketIndicesOffset,
            bucketIndexCount,
          ),
          indexCount: bucketIndexCount,
          vertexCount: bucketVertexCount,
          pointCount: bucketPointCount,
        };
      }
    } else {
      // 单桶：直接复用 record 里的基础索引，格式与 v1 完全兼容。
      buckets = [{ indices, indexCount, vertexCount, pointCount }];
    }
    // 逐帧桶下标（u8）：长度 = frameCount，仅多桶时写入；单桶时为 null。
    const frameBuckets =
      flags & 8 && frameBucketOffset
        ? new Uint8Array(buffer.buffer, buffer.byteOffset + frameBucketOffset, meta.frameCount)
        : null;

    // 汇总成消费端直接可用的网格对象：depth（+24）与 halfThickness（+28）
    // 在此处一并读出，transformIndex（+56）/ renderOrder（+60）同理。
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
      frameOffsets,
      frameBuckets,
      buckets,
      indices,
      colors,
      transformIndex: view.getUint32(record + 56, true),
      renderOrder: view.getUint32(record + 60, true),
      name,
    };
  }
  // 按 renderOrder 升序排序：renderOrder = ((depth + 1) * 10000) + 层内下标，
  // 排序后数组下标即渲染顺序，也是 three.js 侧建网格的顺序。
  meshes.sort((a, b) => a.renderOrder - b.renderOrder);

  // 扁平节点列表（稳定顺序）供 UI / 外部工具使用；id 即 meshes 的下标，
  // 因此 Stage 的 setSelection / 显隐 / 柔光接口可以直接按 id 索引。
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

/**
 * 实例化转换器（每个调用方只需加载一次，实例内部共享同一块 wasm 内存）。
 *
 * @param source 可选；wasm 的 URL / 路径 / 字节 / Module，省略时用默认位置
 * @returns 含 convert / convertBytes / parseModel / version 的对象
 */
export async function loadConverter(source) {
  const module = await readWasm(source);
  const result = await WebAssembly.instantiate(module, {});
  // 传入 Module 时返回 Instance，传入字节时返回 { module, instance }，这里统一取值。
  const instance = result instanceof WebAssembly.Instance ? result : result.instance;
  const exports = instance.exports;
  // 后续所有指针/偏移都基于这块线性内存；memory 可能在增长后换 buffer，需每次取最新视图。
  const memory = exports.memory;
  if (!memory) {
    throw new Error("wasm 实例没有导出 memory");
  }

  const encoder = new TextEncoder();

  /**
   * 只返回 `.l2d3` 原始字节（不解析），适合 CLI 落盘或调用方自行缓存。
   *
   * C ABI 调用顺序与内存释放：
   *   1. l2d_alloc(jsonLen) 分配 JSON 缓冲 → 写入 UTF-8 JSON；
   *   2. 选项存在时再 l2d_alloc(optionLen) → 写入 UTF-8 JSON 选项；
   *   3. l2d_convert(jsonPtr, jsonLen, optPtr, optLen)，status = 0 表示成功；
   *   4. finally 中对两个输入缓冲各调用一次 l2d_dealloc（必须成对，异常时也不泄漏）；
   *   5. status != 0：读 l2d_error_ptr()/l2d_error_len()，然后 l2d_release() 清空输出；
   *   6. status == 0：读 l2d_output_ptr()/l2d_output_len()，slice() 拷贝出 wasm 内存后 release。
   *
   * @param lottie Lottie JSON 字符串或可直接 JSON.stringify 的对象
   * @param options 转换选项（fps / max_frames / thickness / edge_radius / curve_spacing）
   * @returns `.l2d3` 字节（已脱离 wasm 内存，可长期持有）
   */
  function convertBytes(lottie, options = {}) {
    // 输入既可以是已序列化的 JSON 字符串，也可以是普通对象。
    const json = typeof lottie === "string" ? lottie : JSON.stringify(lottie);
    const jsonBytes = encoder.encode(json);
    // 选项按 UTF-8 JSON 传给 wasm；空对象时不额外分配。
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

    // 默认非 0 = 失败；无论 convert 是否抛异常，输入缓冲都要归还。
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
      // 失败：wasm 侧保存了最近一次错误文本，读出后再释放输出缓冲。
      const message =
        readString(memory, exports.l2d_error_ptr(), exports.l2d_error_len()) || "转换失败";
      exports.l2d_release();
      throw new Error(message);
    }

    // 成功：slice() 必须先于 l2d_release() —— release 会回收 wasm 内的输出缓冲，
    // 之后建立的视图会失效或读到脏数据。
    const outputPtr = exports.l2d_output_ptr();
    const outputLen = exports.l2d_output_len();
    const bytes = new Uint8Array(memory.buffer, outputPtr, outputLen).slice();
    exports.l2d_release();
    return bytes;
  }

  /** 转换并解析：convertBytes 出字节，parseModel 包装成 TypedArray 模型。 */
  function convert(lottie, options = {}) {
    return parseModel(convertBytes(lottie, options));
  }

  // parseModel 不依赖 wasm，因此也一并挂到实例上，方便调用方只保存一个对象。
  return {
    convert,
    convertBytes,
    parseModel,
    version: exports.l2d_version(),
  };
}

export default loadConverter;
