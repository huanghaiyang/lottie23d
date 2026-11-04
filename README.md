# lottie23d

把 Lottie（Bodymovin 导出）矢量动画**自动转换**为可交互的 3D 网格，并由 three.js 实时播放。

核心转换器完全用 Rust 编写，同一份代码通过 WebAssembly 同时服务于浏览器和 Node.js，也可以直接作为 Rust 库/命令行工具使用。

```
Lottie JSON ──► Rust 转换器 (wasm) ──► .l2d3 二进制模型 ──► three.js 播放
                        │
                        └──► lottie-web 原画（左侧对照面板）
```

## 特性

- **自动 2.5D 挤出**：填充与描边自动生成"前后盖 + 侧墙"的薄板实体，无需人工建模。
- **完整 Lottie 求值**：位置 / 旋转 / 缩放 / 透明度 / 颜色 / 路径形变（morph）/ 修剪路径（trim）/ 形状组变换 / 父子级 / 预合成 / 时间伸缩 / 关键帧缓动都按 lottie-web 语义实现。
- **逐帧烘焙**：转换阶段一次性完成三角化与动画采样，运行时只做 typed-array 拷贝，零 JSON 解析、零三角化。
- **固定拓扑形变**：顶点数固定，逐帧重采样写入，形变不重建缓冲区。
- **实时参数**：厚度、层深、边缘平滑、采样帧率可在右侧面板实时调整并立即看到效果。
- **左右联播**：左侧 lottie-web 原画、右侧 three.js 3D，同步播放便于对照。

## 环境要求

- Node.js 18+
- Rust（stable）与 `wasm32-unknown-unknown` target

```bash
rustup target add wasm32-unknown-unknown
```

## 快速开始

```bash
npm install
npm run build:wasm     # 编译 Rust → public/wasm/lottie2mesh.wasm
npm run dev            # 启动前端（默认 http://localhost:5173）
```

打开页面即可看到「星轨」示例的左右联播。支持顶部切换内置样例、或把 Lottie JSON 拖入页面导入。

拖入（或顶部「导入 JSON」选择）的文件会自动写入项目根目录的 `data/` 资源库；
顶栏「资源库」入口支持分页浏览、点击预览、逐个载入、批量改名、删除与批量转换校验，
适合一次拖入大量文件后逐个回归。

## 构建

```bash
npm run build:wasm     # 只构建 wasm
npm run build          # vue-tsc 类型检查 + vite 生产构建（输出 dist/）
npm run preview        # 预览生产构建
```

`npm run build` 会自动复制 `public/` 下的样例动画与 wasm 到 `dist/`。

## 在 Node.js 中调用（可嵌入其它项目）

```js
import { loadConverter } from 'lottie2mesh';   // 或相对路径 ./sdk/index.mjs

const converter = await loadConverter();
const model = converter.convert(lottieJson, {
  fps: 30,             // 采样帧率，默认取文档帧率
  max_frames: 600,     // 烘焙帧上限
  thickness: 8,        // 挤出半厚度（Lottie 像素）
  edge_radius: 3,      // 边缘平滑半径（仅填充，会被 clamp 到半厚度）
  curve_spacing: 3,    // 轮廓采样间距（像素/点，1–8；越小曲线越平滑）
});

console.log(model.width, model.height, model.frameCount, model.meshCount);
// 直接可用的 typed arrays：positions / framePositions / indices / transforms …
```

`convert()` 返回的模型结构（见 [sdk/index.d.mts](sdk/index.d.mts)）：

| 字段 | 说明 |
|---|---|
| `width` / `height` / `frameRate` / `ip` / `op` | 文档尺寸与时间范围 |
| `sampleFps` / `frameCount` | 实际烘焙的采样率与帧数 |
| `transforms` | 每个变换轨道：`Float32Array`，逐帧列主序 4×4 世界矩阵 |
| `meshes[]` | 网格数组：`positions`、`framePositions`、`indices`、`colors`、`transformIndex`、`depth`、`halfThickness` 等 |
| `totalVertices` / `totalIndices` / `totalSize` | 统计信息 |

`framePositions` 为 `frameCount × vertexCount × 3`，可直接 `BufferAttribute.set`；`transforms` 与 `THREE.Matrix4.fromArray` 完全兼容。

### CLI

```bash
node sdk/cli.mjs input.json output.l2d3 --fps=30 --max-frames=600 --thickness=8 --edge-radius=3 --curve-spacing=3
```

不传输出路径时会把二进制模型输出到 stdout。

### Rust 原生

```bash
cd rust/lottie2mesh
cargo run --release --bin lottie2mesh-cli -- input.json output.l2d3 --fps=30
```

也可以直接依赖该 crate：

```rust
use lottie2mesh::compile::{compile, RawOptions};

let model = compile(&json, RawOptions { fps: Some(30.0), ..Default::default() })?;
let bytes = lottie2mesh::output::encode(&model);
```

## 转换参数

| 参数 | 位置 | 默认值 | 说明 |
|---|---|---|---|
| `fps` | SDK / CLI / 右侧面板 | 文档帧率 | 烘焙采样率，范围 6–120 |
| `max_frames` | SDK / CLI | 600 | 烘焙帧上限（1–1200），超出会降低采样率 |
| `thickness` | SDK / CLI | 画布最大边的 1.2% | 挤出半厚度（Lottie 像素），限制在 0.01–8% |
| `edge_radius` / `edgeRadius` | SDK / CLI / 右侧面板 | 0 | 填充边缘平滑半径，会被 clamp 到半厚度 |
| `curve_spacing` / `curveSpacing` | SDK / CLI / 右侧面板 | 3 | 轮廓采样间距（像素/点，1–8），越小曲线越平滑、顶点越多 |
| 厚度倍率 | 右侧面板 | 1× | 渲染期沿 Z 缩放，无需重新转换 |
| 层深 | 右侧面板 | 1 | 渲染期放大层间 Z 间距 |
| 采样帧率 | 右侧面板 | 文档帧率 | 修改后自动重新转换并保持播放进度 |

## 支持的 Lottie 能力

- 形状图层：`sh` 路径、`rc` 矩形、`el` 椭圆、`sr` 多角星、`gr` 组、`tr` 组变换、`mm` 合并路径（并集 / 相减 / 相交 / 排除）
- 样式：`fl` 纯色填充、`st` 描边、`gf` 渐变填充（逐顶点采样线性/径向渐变）、`gs` 渐变描边
- 动画：位置（含分离维度 `p.s`）、旋转、缩放、锚点、透明度、颜色、路径 morph、trim、组变换、预合成、时间伸缩、父子级
- 缓动：`i/o` 三次贝塞尔（含按维度取值）、`h` 保持关键帧、空间贝塞尔 `to/ti`（150 段弧长采样）
- 轨道遮罩：`tt` + `td` 图层的 alpha 遮罩（含反转）与预合成父子继承；静态遮罩只裁一次，动态遮罩逐帧重建
- 图片图层：`ty:2` 资源按 `u + p`（或内嵌 data URI）作为贴图平面渲染，支持图层变换与透明度动画
- 图层遮罩（`masksProperties`）：静态遮罩并进轮廓，带关键帧的遮罩逐帧裁剪

## 已知限制

- **2.5D 而非真 3D**：Lottie 本身是 2D 合成格式，Z 深度按图层顺序自动分层；3D 图层的 `rx/ry`、透视相机、真实 Z 位置不还原（只使用 `rz`）。
- **未支持图层**：文本图层（`ty:5`）、表达式、效果器。
- **轨道遮罩细节**：luma 模式（`tt:3/4`）按 alpha 近似处理；遮罩源目前仅支持形状 / 纯色图层，预合成图层作为遮罩源会退化为无裁剪；遮罩层的部分透明度不参与裁剪（按轮廓二值化）；图片图层不受轨道遮罩裁剪。
- **图片资源**：纹理按页面相对路径加载（如 `/images/3.png`），需要资源可访问；加载失败时该图层直接跳过，不影响其它图层。
- **形变边界**：路径 morph 要求轮廓数量与闭合性不变；顶点数不同时按 lottie 语义退化为切换而非插值。
- **渐变**：颜色与透明度 stop 均已按顶点位置采样；动画渐变只采样参考帧，stop 关键帧变化不会逐帧更新。
- **同一合成内 `ind` 必须唯一**，否则父级引用会错乱（Lottie 规范要求唯一）。

更深入的设计说明、二进制格式与求值细节见 **[docs/technical.md](docs/technical.md)**。

## 目录结构

```
.
├─ rust/lottie2mesh/          Rust 转换器 crate
│  ├─ src/lottie.rs           文档结构定义（serde）
│  ├─ src/prop.rs             关键帧与缓动求值
│  ├─ src/path.rs             贝塞尔路径求值 / trim / 展平
│  ├─ src/shape.rs            形状树求值（含组变换矩阵）
│  ├─ src/mesh.rs             三角化、描边、挤出、边缘圆角、逐帧写点
│  ├─ src/compile.rs          图层/预合成遍历、逐帧烘焙
│  ├─ src/output.rs           .l2d3 二进制编码
│  ├─ src/lib.rs              C ABI（alloc/convert/output/release/error）
│  └─ src/bin/cli.rs          原生命令行
├─ scripts/
│  ├─ build-wasm.mjs          Rust → wasm 构建脚本
│  └─ library-plugin.mjs      资源库中间件（上传写入 data/，仅 dev / preview）
├─ sdk/
│  ├─ index.mjs               加载 wasm、转换、解析二进制模型
│  ├─ index.d.mts             类型声明
│  └─ cli.mjs                 Node CLI
├─ src/                       Vue 3 前端
│  ├─ App.vue                 页面编排、播放循环、参数与重转换
│  ├─ components/             TopBar / TransportBar / InspectorPanel
│  ├─ scene/stage.ts          three.js 播放器（逐帧上传、脏渲染、质量自适应）
│  ├─ samples.ts              内置 Lottie 样例
│  └─ lottie/types.ts         Lottie 类型
├─ public/
│  ├─ wasm/lottie2mesh.wasm   构建产物
│  └─ animations/             样例 Lottie 文件
├─ data/                      资源库：页面里上传的 Lottie（本地生成，已忽略）
└─ docs/technical.md          技术文档
```
