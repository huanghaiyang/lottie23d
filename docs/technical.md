# lottie23d 技术文档

本文说明 lottie23d 的内部设计：Lottie 求值语义、几何管线、二进制模型格式、three.js 播放器与性能取舍。面向需要二次开发、排查转换偏差或把转换器嵌入其它项目的开发者。

- 项目入口与使用方式见 [README](../README.md)
- Rust 源码位于 [`rust/lottie2mesh/src`](../rust/lottie2mesh/src)
- JS SDK 位于 [`sdk`](../sdk)，前端位于 [`src`](../src)

---

## 1. 设计目标

| 目标 | 做法 |
|---|---|
| 浏览器与 Node 共用一套转换器 | Rust 编译为 `wasm32-unknown-unknown`，导出纯 C ABI；JS 侧只负责内存搬运与二进制解析 |
| 运行时零转换成本 | 所有几何/三角化/关键帧采样在转换阶段完成，播放阶段只写矩阵与顶点数组 |
| 动画不重建缓冲区 | 参考帧确定固定拓扑，逐帧按弧长重采样并覆盖同一批顶点 |
| 可嵌入其它 Node 项目 | `sdk/index.mjs` 无构建依赖，直接 `import`；也提供 CLI 与 Rust 库接口 |
| 2.5D 观感 | 按图层顺序生成 Z 深度，填充/描边挤出为带前后盖与侧墙的薄板 |

---

## 2. 分层架构与数据流

```text
                 ┌────────────────────────────────────────────┐
Lottie JSON ───► │ Rust core (compile / shape / path / mesh)  │
                 └───────────────────┬────────────────────────┘
                                     │ output::encode
                                     ▼
                          .l2d3 二进制模型（TypedArray 布局）
                                     │
              ┌──────────────────────┴───────────────────────┐
              │                                              │
              ▼                                              ▼
   sdk/index.mjs（Node / 浏览器）                    Vue + lottie-web 原画
   loadConverter → convert → parseModel              （左侧对照面板）
              │
              ▼
   src/scene/stage.ts（three.js 播放器，右侧面板）
```

分层职责：

1. **Rust core**：JSON 解析（serde）、时间模型、关键帧求值、形状树求值、三角化与挤出、逐帧烘焙。
2. **C ABI**（[`lib.rs`](../rust/lottie2mesh/src/lib.rs)）：只暴露内存分配、转换、结果读取、错误读取与版本号。
3. **JS SDK**（[`sdk/index.mjs`](../sdk/index.mjs)）：加载 wasm、序列化 JSON/选项、把二进制 blob 包装成 `Float32Array` / `Uint32Array`。
4. **播放器**（[`src/scene/stage.ts`](../src/scene/stage.ts)）：固定 `BufferGeometry`，逐帧写 `matrixWorld` 与顶点，脏渲染。
5. **页面**（[`src/App.vue`](../src/App.vue)）：播放时钟、双侧同步、参数面板与实时重转换。

### 2.1 C ABI 调用顺序

```text
len  = l2d_alloc(jsonBytes.length)
write json into wasm memory
opt  = l2d_alloc(optionsBytes.length)        // 可选
status = l2d_convert(jsonPtr, jsonLen, optPtr, optLen)   // 0 = 成功
bytes    = new Uint8Array(l2d_output_ptr(), l2d_output_len())
message  = l2d_error_ptr() / l2d_error_len() // status != 0 时读取
l2d_release()                                // 释放输出
l2d_dealloc(jsonPtr, jsonLen)
```

`options` 是一段 UTF-8 JSON：

```json
{ "fps": 30, "max_frames": 600, "thickness": 8, "edge_radius": 3, "curve_spacing": 3 }
```

字段均可省略；`edge_radius` / `curve_spacing` 也接受别名 `edgeRadius` / `curveSpacing`。`l2d_version()` 当前返回 `2`，ABI 或二进制格式变化时递增。

---

## 3. 时间模型与采样

### 3.1 帧数与采样率

```text
raw_frames = round(((op - ip) / frame_rate) * sample_fps)
sample_fps = clamp(options.fps ?? frame_rate, 6, 120)
max_frames = clamp(options.max_frames ?? 600, 1, 1200)
frame_count = clamp(raw_frames, 1, max_frames)
```

### 3.2 根采样时刻

第 `i` 帧（`i = 0 … frame_count - 1`）的根时间为：

```text
root_frame = ip + (op - ip) * (i / frame_count)
```

注意采样不包含右端点 `op`，因此不会出现首尾重复帧；这也是 `sample_fps` 会按 `frame_count / ((op-ip)/frame_rate)` 反算的原因。

### 3.3 图层本地时间

- 普通图层的属性与 `ip` / `op` 都在**父合成时间轴**上求值，图层自身的 `st` 不参与：lottie-web 的 `getValueAtCurrentTime` 里 `frameNum = comp.renderedFrame - offsetTime`（`offsetTime` 即 `st`），而关键帧时间也按同一 `offsetTime` 平移，两边相消。
- 预合成子层：`st` 只用于进入子合成时换算时间轴，子层自身的 `st` 同样相消：

```text
time remap 启用时：child_parent_frame = tm(owner_local) * frame_rate   // tm 以秒计
否则：             child_parent_frame = (root_frame - owner.st) / sr
```

`tm` 优先于 `sr`（与 lottie-web 的 `CompElement.prepareFrame` 一致）。`sr` 为 0 时按 1 处理。参考帧的选择同样遵循该规则，并 clamp 到子层 `ip … op - 0.001`。

`tm` 带 `loopOut('cycle')` 表达式时（lottie-web 自带表达式插件，会实际求值）：

```text
comp_frame > last_keyframe.t 时：tm 的求值帧 = first_t + (comp_frame - first_t) % (last_t - first_t)
```

即超出末关键帧后子合成时间循环回起点，子合成会反复播放；`pingpong` / `offset` / `continue` 暂未实现。

`starfish.json` 的 `eye` 预合成同时用到这两点：`tm` 把外层 0…140 帧映射到子合成 0…140 帧（末关键帧 2.333 s ≈ 139.98 帧），`loopOut('cycle')` 再让它按 140 帧周期循环；子合成里的眼睑遮罩在 26…44 帧滑动，于是整段动画里眼睛在第 26…44、166…184 帧各眨一次，而不是眨完就消失或一直不动。

---

## 4. 关键帧与属性求值

实现位于 [`prop.rs`](../rust/lottie2mesh/src/prop.rs)，语义对齐 lottie-web 的 `PropertyFactory`。

- 关键帧判定不看 `a`：按 `k` 的**首元素类型**区分——首元素是数字（或 `k` 为空）为静态值，否则为关键帧数组。这与 lottie-web 的 `PropertyFactory.getProp` 一致；旧版 Bodymovin 导出（如 `dalek.json` 的 3.1.6）的 `ks` 属性普遍缺少 `a`，只看 `a` 会把整套动画误判成静态。
- 静态属性：直接读取 `k`（数字、数组或带 `v`/`s` 的对象）。
- 动画属性：在 `k[]` 中定位 `t ≤ frame` 的区间。
- 结束值：`next.s`，缺失时回落到当前关键帧的 `e`（对应 lottie-web 的 `nextKeyData.s || keyData.e`）。
- 保持帧：`h === 1` 时整段返回起始值。
- 缓动：`i.x/i.y/o.x/o.y` 支持标量或按维度数组；用二分求解三次贝塞尔的 X 轴，再取 Y 值（18 次迭代）。
- 空间贝塞尔：位置关键帧带 `to` / `ti` 时，以 `s + to`、`e + ti` 为控制点构建三次曲线，固定 **150 段**采样，按累计弧长取目标进度（与 `bez.buildBezierData` 一致）。
- 分离维度：`p.s === true` 时分别求值 `p.x` / `p.y` / `p.z`。

### 4.1 路径 morph

`sh` 的 `ks` 求值在 [`path.rs`](../rust/lottie2mesh/src/path.rs)：

- 静态路径与动画关键帧都有两种存储形式：路径对象本身，或只含一个路径对象的数组（`s[0]`）。统一由 `parse_path_value` 解包。
- 顶点数一致时，对 `v/i/o` 逐分量插值；顶点数不一致时按 lottie 语义**退化为切换**（`progress < 0.5` 取前，否则取后）。
- `i` / `o` 缺省时补零向量；闭合标志由 `c` 决定。

---

## 5. 变换矩阵约定

这是整个转换器最容易出错的部分，务必区分两种形式。

### 5.1 lottie-web：行向量、行主序

lottie-web 的 `Matrix` 以行向量参与运算，平移是后乘（`_t`），旋转调用 `rotate(-r)`。

### 5.2 本转换器：列向量

`Affine { a, b, c, d, tx, ty }` 作用于列向量：

```text
x' = a * x + c * y + tx
y' = b * x + d * y + ty
```

`mul(self, other)` 表示 `self * other`，即 `other` 先作用。转置后旋转符号翻转，所以实现中调用 `Affine::rotate(+r)`。代码见 [`shape.rs`](../rust/lottie2mesh/src/shape.rs) 的 `transform_matrix`。

### 5.3 合成顺序

```text
M = T(p) · R(+r) · Skew · S · T(-a)
Skew = R(sa) · Shear(tan(-sk)) · R(-sa)
```

- 3D 图层：存在 `rx` 时优先使用 `rz`（纯 2D 旋转），否则用 `r`。
- 形状组：组内 `tr` 无论出现在列表哪个位置，都作用于该组全部内容。
- 图层链：`world = parent_world · local`；顶层再左乘全局矩阵。
- 全局矩阵把 Lottie 画布映射到 three.js 空间：`scale = 3.6 / max(w, h)`，Y 轴翻转，画布中心移到原点，Z 使用同一比例。

### 5.4 深度烘焙

每层分配一个 Z 深度：

```text
顶层：depth = 0.5 - index / (count - 1)
子层：depth = owner.depth + (0.5 - index / (count - 1)) * 0.02
两者都 clamp 到 [-0.65, 0.65]
```

子层的展开范围只有 ±0.01，必须远小于相邻顶层间距（约 0.022），否则子合成会跨过相邻图层（`dalek.json` 的 `rayitos` 曾因此被压到蓝色背景之后）。

深度直接加在最终矩阵的 Z 平移分量（列主序第 14 个元素），所以播放器可以原样上传矩阵；右侧“层深”滑块只是追加一个增量。**子层只累加相对 owner 的深度差**——子层的 `base` 矩阵（owner 世界矩阵）里已经含了 owner 的深度平移，再叠加绝对深度会被算两次（`dalek.json` 的 `rayitos` 因此被推到 `z ≈ -0.97`，整片闪电被不透明的蓝色背景挡住）。

`render_order = ((depth + 1) * 10000) + index`，SDK 解析后按它升序排序网格数组。

**播放器侧透视补偿**：舞台用 40° 透视相机（距原点 7.2）。若直接投影，层深差会让各层在平移时产生视差与大小差——离画面中心越远越明显，同一帧内可达 60 像素，与原画对不上。因此渲染期在相机空间对每层做一次以相机为原点的 XY 缩放：

```text
k = 图层沿视线方向的深度 / 相机到观察目标的距离
```

投影因此退化为统一尺度的正交投影：任意视角下各层位置都与 Lottie 原画重合；层深仍保留在 Z 上负责遮挡与排序，绕转视角时也照常显出前后错位（错位来自旋转而非透视除法）。代价是前视图里“层深”滑块只剩遮挡次序变化。

---

## 6. 可见性与透明度

- 可见性判定使用**父合成帧**（即图层属性求值用的同一个帧）：`ip ≤ frame < op` 时可见，`op <= ip` 视为始终可见；图层自身的 `st` 在 `ip - st` 与 `frame - st` 两边相消，与 lottie-web 的 `checkLayerLimits` 等价。
- 透明度沿合成树相乘：`opacity = owner_opacity * own_opacity`。
- 只有当颜色或透明度确实变化时才分配逐帧颜色轨道；常量颜色会把第 0 帧的图层透明度直接折进网格记录。
- 触发动态颜色的条件包括：样式动画、图层 `o` 动画、父级透明度动画、父级存在时间伸缩/位移，或在采样区间内并非全程可见。

---

## 7. 形状求值与几何管线

[`shape.rs`](../rust/lottie2mesh/src/shape.rs) 把嵌套形状树展平为 `Drawable` 列表：

- 支持 `sh` 路径、`rc` 矩形、`el` 椭圆、`sr` 多角星、`gr` 组、`tr` 组变换、`tm` 修剪。
- 支持 `fl` 纯色填充、`st` 描边、`gf` / `gs` 渐变（按首色近似）。
- 形状变换会烘焙进轮廓坐标，运行时不再做组变换。
- `eval_styles` 只求颜色，跳过路径展平与组变换，用于“仅样式动画”的场景。
- 颜色范围按数值判定：旧版 Bodymovin（`v` 低于 4.1.9，如 `dalek.json` 的 3.1.6）的 `fl` / `st` 用 0..255，新版用 0..1。任一通道大于 1 即整组除以 255（与 lottie-web 的 `checkColors` 一致），否则渲染端会把 0..255 的颜色钳成白色剪影。
- 图元顶点顺序必须与 lottie-web 一致，因为 `tm` 修剪的起点就是路径起点：椭圆从正上方开始（顺时针为上→右→下→左，`d = 3` 时为上→左→下→右），矩形从右上角开始顺时针。顺序不一致会让光弧/扇形整体偏转 90°（`pulse` 的三条环绕光弧曾如此）。

### 7.0 图层遮罩

`masksProperties` 只实现了**带关键帧的单条遮罩**：`mode = "a"`（加法）、未反相、遮罩轮廓为凸多边形。求值时把遮罩路径展平，再用 Sutherland–Hodgman 逐帧裁剪该层所有 drawable 的轮廓（多边形求交）。瞬态遮罩（`o`、`x`）不参与计算。

- 遮罩路径有关键帧时该层被标记为几何动画，逐帧重新求值并裁剪；被完全裁掉的轮廓会被丢弃，整个 drawable 被裁空时该帧不写几何。
- 静态遮罩仍按旧行为忽略（例如 Dalek 的静态裁剪），避免改变已有示例外观。
- 裁剪后轮廓的采样点数会变化，折叠/遮罩产生的拓扑桶只适合当时的姿态：烘焙时按“与当帧自然采样点数最接近”的桶写帧（差距超过 25% 才切换），避免形状恢复后仍用低采样桶把轮廓画成多边形。

`starfish.json` 的眼睛眨眼就是这一机制：子合成里的 `Mask 1` 是 36.75 × 20.75 的矩形，在 26…44 帧先从眼睛上滑走再滑回，眼睛因此被裁成一条细缝再重新露出。

### 7.1 轮廓与重采样

- 路径按自适应细分展平为折线（对齐 `bez.getSegmentsLength` 的思路）。
- 参考帧为每个轮廓确定固定采样数：

```text
k = clamp(ceil(arc_length / curve_spacing), 3, 8192)   // 退化轮廓用 3
```

- 采样密度由转换选项 `curve_spacing`（默认 3 像素/点，范围 1–8）决定：播放器会把画布放大到 3.6 个世界单位，4 像素以上的弦长在放大视图里会出现可见折线。`k` 不再按 480 点截断，长轮廓也会按真实间距采样；下限 3 保证小轮廓（周长十几像素的灯泡）在间距调大时同样能真实降采样；8192 点只是极端路径的显存兜底，外部参数不会因此失真。

- 逐帧用 `resample_into` 按弧长重新采样到同一个 `k`，保证顶点数不变。
- 修剪路径的环绕区间被拆成固定的两个槽位（`trim_slots`），避免拓扑在环绕时跳变。
- 若某一帧的轮廓数量少于参考帧，缺失槽位退化为单点轮廓（几何塌陷）；多于参考帧的部分被忽略，即形变需要轮廓数量与闭合性稳定。

### 7.2 填充

[`mesh.rs`](../rust/lottie2mesh/src/mesh.rs)：

1. 按 `|面积|` 降序排列轮廓，用代表点 + 点在多边形内测试做包含分组，识别外轮廓与孔洞。
2. 每个“外轮廓 + 孔洞”组交给 `earcutr` 三角化。
3. 代表点优先取质心，质心落在多边形外时退化为首条边中点。

### 7.3 描边

- 沿平均法线向两侧偏移 `width / 2`。
- 斜接限制：`miter = min(half / dot, half * miterLimit)`，`dot` clamp 到 `[0.2, 1]`，避免尖角爆炸。
- 生成三角形条带；闭合轮廓首尾相接。

### 7.4 挤出

平面三角形表面被扩展为封闭薄板：

- 前盖：原三角形。
- 后盖：顶点顺序反转、使用剖面最后一环。
- 侧墙：统计无向边，只出现一次的边即边界边，逐环生成四边形。

### 7.5 边缘平滑

- 仅填充生效，半径 clamp 到 `[0, half_thickness]`，描边始终使用锐边。
- `stride = 2`（锐边）或 `8`（平滑：上下各 3 段四分之一圆弧 + 2 个平直环）。
- 环剖面沿内法线偏移 `radius * (1 - cos θ)`，Z 为 `(half - radius) + radius * sin θ`。
- 外轮廓向内部偏移，孔洞方向翻转，保证实体不越过边界。

### 7.6 逐帧写点

`write_frame` 与 `build_surface` 共用同一套 `SlotPlan`：

1. 对每个槽位按同一 `k` 重采样。
2. 填充直接拼接；描边重建 ribbon 点。
3. 按径向剖面展开成 `point_count × stride` 个顶点，直接写入目标帧切片。
4. 复用 `FrameScratch`，整个烘焙过程不按帧分配内存。

---

## 8. 烘焙流程

[`compile.rs`](../rust/lottie2mesh/src/compile.rs) 的 `compile`：

1. 解析文档，读取 `w/h/fr/ip/op`（缺失或非法时使用 512 / 30 / 3 秒兜底）。
2. 计算采样参数、厚度与边缘半径。
3. 递归遍历图层：形状层（`ty:4`）、纯色层（`ty:1`）、预合成（`ty:0`）。
4. 在参考帧调用 `shape::eval_shapes` 了解可绘制对象 → `mesh::build_surface` 得到参考网格与固定拓扑。
5. 若存在几何动画，分配 `frame_count × vertex_count × 3` 的顶点轨道。
6. 全局 float 预算：动画顶点总 float 超过 **8,000,000**（约 32 MB）时，按 `(8M / floats).max(24)` 降采样，但至少保留 24 帧。
7. 逐帧烘焙：
   - 用缓存递归计算每层世界矩阵与透明度，写入变换轨道。
   - 仅几何动画时求 `eval_shapes`；仅样式动画时求 `eval_styles`；两者都没有则跳过求值。
   - 动态颜色写颜色轨道；静态颜色在第 0 帧折算图层透明度。
   - 几何动画写入帧顶点轨道。
8. 生成 `render_order`，校验网格非空。

转换耗时（`conversion_us`）只在原生构建记录；wasm 构建为 0。

---

## 9. `.l2d3` 二进制格式

编码器见 [`output.rs`](../rust/lottie2mesh/src/output.rs)。所有数据小端；偏移均为从 blob 起始的绝对字节偏移。

```text
┌──────────────────────────────┐
│ header           96 B        │
├──────────────────────────────┤
│ mesh records     96 B × N    │
├──────────────────────────────┤
│ transform tracks 16 f32 × F × T │
├──────────────────────────────┤
│ data chunks：静态顶点 / 帧顶点 / 索引桶 / 颜色 / 桶表 / 帧偏移 / 网格名 │
├──────────────────────────────┤
│ 文档名（4 字节对齐）          │
└──────────────────────────────┘
```

### 9.1 Header（96 B）

| 偏移 | 类型 | 含义 |
|---|---|---|
| 0 | u32 | magic `0x4C323344`（"L23D"） |
| 4 | u32 | 版本，当前 `2` |
| 8 / 12 | f32 | 画布宽 / 高 |
| 16 | f32 | 文档帧率 |
| 20 / 24 | f32 | `ip` / `op` |
| 28 | f32 | 实际采样帧率 |
| 32 | u32 | 烘焙帧数 `frame_count` |
| 36 | u32 | 网格数 |
| 40 | u32 | 变换轨道数 |
| 44 | u32 | 总顶点数 |
| 48 | u32 | 总索引数 |
| 52 | u32 | 文件总字节数 |
| 56 | u32 | 原生转换耗时（微秒，wasm 为 0） |
| 64 | u32 | 文档名偏移 |
| 68 | u32 | 文档名长度 |
| 其余 | — | 保留 |

### 9.2 Mesh record（96 B）

| 偏移 | 类型 | 含义 |
|---|---|---|
| 0 | u32 | `kind`：`0` 填充 / `1` 描边 |
| 4 | u32 | flags：bit0 有帧顶点轨道，bit1 有颜色轨道，bit2 有帧顶点偏移表，bit3 有多个拓扑桶 |
| 8 / 12 / 16 / 20 | f32 | 静态颜色 RGBA（sRGB，0–1） |
| 24 | f32 | Z 深度 |
| 28 | f32 | 挤出半厚度（Lottie 像素） |
| 32 | u32 | 顶点数（所有拓扑桶的最大值） |
| 36 | u32 | 索引数（桶 0） |
| 40 | u32 | 静态顶点偏移（3 × f32） |
| 44 | u32 | 帧顶点偏移（按 `frame_offsets` 连续打包，无则 0） |
| 48 | u32 | 索引偏移（桶 0 的 u32 索引） |
| 52 | u32 | 颜色轨道偏移（`frame_count × 4 × f32`，无则 0） |
| 56 | u32 | 变换轨道索引 |
| 60 | u32 | 渲染顺序 |
| 64 | u32 | 参考轮廓点数（未计 `stride`） |
| 68 / 72 | u32 | 网格名偏移 / 长度 |
| 76 | u32 | 拓扑桶数量 |
| 80 | u32 | 桶表偏移（每桶 16 B：索引偏移 / 索引数 / 顶点数 / 轮廓点数） |
| 84 | u32 | 帧顶点偏移表偏移（`frame_count + 1` 个 u32，单位为 float） |
| 88 | u32 | 帧拓扑桶表偏移（`frame_count` 个 u8） |
| 92 | — | 保留 |

### 9.3 变换轨道

每条轨道是 `frame_count × 16` 个 f32，**列主序 4×4**，可直接传给：

```js
matrix.fromArray(track, frame * 16);
// 或 three.js：
mesh.matrixWorld.fromArray(track, frame * 16);
```

与 `THREE.Matrix4.elements` 完全兼容。

### 9.4 数据块

- 参考顶点：`vertex_count × 3` f32，位于图层局部坐标（已烘焙组变换与挤出 Z），最终位置由变换轨道中的世界矩阵决定。
- 帧顶点：按帧连续打包，`frame_offsets[f]` 到 `frame_offsets[f + 1]`（单位 float）是本帧的顶点区间；不同拓扑桶的顶点数可以不同。
- 索引桶：每桶一份 `index_count` u32 索引；桶表记录各桶的偏移、索引数、顶点数与轮廓点数。
- 颜色轨道：`frame_count × 4` f32（RGBA，sRGB）。
- 字符串：UTF-8，尾部补零到 4 字节对齐。

---

## 10. JS SDK

[`sdk/index.mjs`](../sdk/index.mjs) 导出：

| 接口 | 说明 |
|---|---|
| `loadConverter(source?)` | 加载 wasm。缺省 Node 读 `public/wasm/lottie2mesh.wasm`，浏览器读 `document.baseURI + wasm/lottie2mesh.wasm`；也接受路径、`ArrayBuffer`、`TypedArray`、`WebAssembly.Module` |
| `converter.convert(json, options?)` | 转换为已解析的模型对象 |
| `converter.convertBytes(json, options?)` | 只返回 `.l2d3` 原始字节，适合落盘或缓存 |
| `converter.parseModel(bytes)` | 把二进制解析为 TypedArray（不依赖 wasm） |
| `converter.version` | ABI 版本 |

解析结果：

```ts
{
  version, width, height, frameRate, ip, op, sampleFps,
  frameCount, meshCount, transformCount,
  totalVertices, totalIndices, totalSize, conversionUs,
  name, duration,
  transforms: Float32Array[],       // 每条 frameCount × 16
  meshes: MeshModel[],              // 已按 renderOrder 排序
}
```

`MeshModel` 包含 `positions`、`framePositions`、`frameOffsets`、`frameBuckets`、`buckets`、`indices`、`colors`、`transformIndex`、`renderOrder`、`depth`、`halfThickness`、`kind`、`name` 等字段（完整声明见 [`sdk/index.d.mts`](../sdk/index.d.mts)）。`buckets[0].indices` 与 `indices` 相同，便于只关心单一拓扑的调用方。

浏览器端部署时，把 `public/wasm/lottie2mesh.wasm` 放到站点根目录的 `wasm/` 下即可；也可显式传入 URL。

---

## 11. three.js 播放器

[`stage.ts`](../src/scene/stage.ts) 的关键设计：

### 11.1 固定几何

- 每个网格只创建一次 `BufferGeometry`；动画网格使用 `DynamicDrawUsage`。
- 矩阵关闭自动更新：`matrixAutoUpdate = false`、`matrixWorldAutoUpdate = false`、`frustumCulled = false`。
- 材质为 `MeshLambertMaterial` + `flatShading` + `DoubleSide`。

### 11.2 逐帧上传

```text
matrixWorld.fromArray(track, frame * 16)
elements[14] += depth * (depthScale - 1)      // 层深滑块
elements[2/6/10] *= thicknessScale            // 厚度滑块（等价后乘 scale(1,1,t)）
position.array.set(framePositions.subarray(frameOffsets[f], frameOffsets[f + 1]))
position.needsUpdate = true
geometry.setIndex(indexAttributes[frameBuckets[f]])   // 拓扑变化时切换
```

- 全程静态的变换轨道只上传一次（`isConstantTrack`）。
- 颜色动画直接写 `material.color.setRGB(..., SRGBColorSpace)` 与 `opacity`。
- 透明度低于 `0.004` 的网格直接 `visible = false`，跳过绘制。
- 每套拓扑桶的索引属性只创建一次；切换到相同桶时不做任何绑定。

### 11.3 脏渲染

- `controls.update()` 返回是否移动；只有移动或 `dirty` 时才 `render`。
- 暂停且视角静止时不产生 GPU 开销。
- FPS 读数节流，避免频繁 DOM 写入。

### 11.4 质量自适应

连续 6 个低帧窗口（`fps < 45`）后降低一次 `pixelRatio`（×0.8，下限 0.75）；恢复流畅后计数清零。

### 11.5 参数实时生效

| 参数 | 处理方式 |
|---|---|
| 厚度倍率 | 只改矩阵 Z 缩放，不重转换 |
| 层深 | 只给 `elements[14]` 加增量，不重转换 |
| 自动环绕 | `OrbitControls.autoRotate` |
| 边缘平滑 | 防抖 90 ms 后重新转换，保持播放进度比例 |
| 采样帧率 | 防抖后重新转换，保持播放进度比例 |

边缘平滑以“半厚度的百分比”暴露：`edge_radius = ratio / 100 * halfThickness`，避免用户在不同画布尺寸下反复试数值。

---

## 12. 性能设计

### 12.1 转换期

- 每个姿态桶只三角化一次；逐帧只做弧长重采样 + 写顶点，遇到姿态锚点、前盖折叠超阈值或轮廓槽位变化才建新桶。
- 每层每帧的世界矩阵与透明度带缓存，父子链不重复计算。
- 复用 `FrameScratch`、世界矩阵缓存与局部帧缓存，烘焙过程零逐帧分配。
- 仅几何动画时才展平路径；仅样式动画时跳过几何。
- 顶点跟随表达式每帧只解析一次空对象位置（`fill_follow_positions`），多个 drawable 共享。

### 12.2 内存预算

| 项 | 约束 |
|---|---|
| 动画顶点 float 总量 | 8,000,000（约 32 MB） |
| 烘焙帧数 | 1–1200，默认 600 |
| 预算降采样下限 | 24 帧 |
| 变换轨道 | 每层 `frame_count × 64 B` |
| 颜色轨道 | 仅动态颜色网格分配 |

转换会优先保证可播放性：先尝试目标帧率，超预算时自动降低帧数，而不是让内存失控。

### 12.3 运行期

- 零 JSON 解析、零三角化、零对象分配（除首次建模）。
- 每帧成本 ≈ 变换矩阵写入 + 动画顶点 `memcpy`。
- 静态网格完全跳过逐帧写点。
- 不透明常量颜色禁用混合，减少排序与 overdraw。
- 脏渲染与自适应 `pixelRatio` 降低空闲/低端设备开销。

---

## 13. 参数汇总

| 参数 | 范围 / 默认 | 作用域 |
|---|---|---|
| `fps` | 6–120，默认文档帧率 | SDK / CLI / 面板 |
| `max_frames` | 1–1200，默认 600 | SDK / CLI |
| `thickness` | 默认 `max(w,h) * 1.2%`，clamp 到 `[0.01, max(w,h) * 8%]` | SDK / CLI |
| `edge_radius` | 默认 0，clamp 到 `[0, half_thickness]`；仅填充 | SDK / CLI / 面板 |
| `curve_spacing` | 1–8 px/点，默认 3；越小越平滑 | SDK / CLI / 面板 |
| 厚度倍率 | 0.25–2.5× | 面板（渲染期） |
| 层深 | 0–2.5 | 面板（渲染期） |
| 采样帧率 | 6–120 | 面板（触发重转换） |

---

## 14. 已知限制与取舍

1. **2.5D**：不还原真实 3D 图层的 `rx/ry` 与 Z 位置；只使用 `rz` 与自动分层深度。透视相机只负责让挤出厚度产生立体感，层间视差在渲染期被补偿掉（见 §5.4）。
2. **不支持图层**：图片（`ty:2`）、文本（`ty:5`）、轨道遮罩、效果器。蒙版只支持带关键帧的单条加法遮罩（见 §7.0），静态遮罩、反相 / 减法 / 相交遮罩仍按旧行为忽略。表达式只识别“顶点跟随空对象”“沿路径分布（Trace Path，含 `tangentOnPath` 切线旋转）”“位置弹性回弹”“时间重映射 `loopOut('cycle')`”四种模式（见 §14.2、§14.3），其余表达式忽略。
3. **形变按桶切换**：姿态锚点、前盖折叠超阈值或轮廓槽位变化时切换索引桶（最多 24 套），桶内按弧长对应；达到上限后沿用当前桶。写帧时还会按“与当帧自然采样点数最接近”的桶做一次选择（差距超过 25% 才切换），避免遮罩 / 折叠产生的低采样桶在形状恢复后继续把轮廓画成多边形。
4. **渐变近似**：`gf` / `gs` 取首色作为纯色。
5. **`ind` 必须唯一**：同一合成内重复 `ind` 会导致父级挂错（lottie-web 按 `ind` 遍历所有图层，重复属于非法数据）。
6. **修剪模式**：忽略 `tm` 的 `m`（同时/逐个）字段，统一按每条轮廓各自的归一化弧长裁剪，等价于“逐个”语义；多轮廓场景与 lottie-web 的“同时”模式可能存在差异。
7. **颜色空间**：Lottie 颜色按 sRGB 解释；SDK 返回原始 sRGB 分量，three.js 侧统一用 `SRGBColorSpace` 转换。

### 14.1 形变能力的三档方案

当前采用 B 档；C 档保留为后续选项：

| 方案 | 做法 | 代价 |
|---|---|---|
| A（基础） | 固定 `k`，弧长重采样 | 顶点对应与 lottie 不完全一致，但零重建、零分配 |
| B（已实现） | 按姿态锚点 + 折叠检测分桶缓存多套 indices，帧间切换索引 | 每桶一份索引显存；切换时重建 `geometry.index`；最多 24 桶 |
| C（未实现） | 每帧重新三角化 | 保真度最高，但每帧 CPU 成本高，不适合浏览器实时播放 |

B 档细节：

- 姿态锚点：采样区间按 1/7 等分，每段起点用当帧几何重新三角化，把剧烈的形变误差限制在一小段内。
- 折叠检测：逐帧计算填充前盖的 `Σ|三角形面积| / |Σ有符号面积|`（1.0 表示绕向一致、互不重叠）。超过 1.015 时立即按当前姿态新建一套三角化并重写本帧顶点，避免裂缝延续到下一帧。描边的环形带有符号面积和接近 0，不参与该判定。
- Delaunay 边翻转：三角化后对同一条轮廓内部的公共边做 Lawson 翻转，缩短横跨凹口的长边，降低形变时三角形反绕的概率；边界边与孔洞连接边保持不变。
- 帧顶点不再按 `frame_count × max_vertex_count` 预留，而是按帧连续打包，由 `frame_offsets` 表定位；因此各桶可以有不同顶点数。
- 帧级 `frame_buckets` 记录本帧使用的桶；只有单桶时不写桶表，格式开销为零。
- 桶内仍按弧长重采样，所以 B 档提升的是“拓扑正确性”，不是逐顶点对应。

### 14.2 顶点跟随空对象表达式

`starfish.json` 这类 AE 导出会把路径顶点绑定到空对象：

```js
var nullLayerNames = [ 'Shape Layer 1: Path 1 [1.0]', ... ];
for (...) origPoints[i] = fromCompToSurface(nullLayer.toComp(nullLayer.anchorPoint));
$bm_rt = createPath(origPoints, origInTang, origOutTang, origPath.isClosed());
```

转换器的处理：

1. `vertex_follow_names` 识别 `fromCompToSurface` + `Layer Control` 表达式，按括号深度解析 `nullLayerNames` 数组（层名本身含 `[1.0]`，不能用第一个 `]` 收尾）。
2. 每帧对每个空对象求 `comp_anchor = plan_chain(null) × anchorPoint`，得到 comp 坐标。
3. 用形状层链条矩阵的逆（`fromCompToSurface`）变回图层局部坐标，替换原路径顶点；切线保持不变。
4. 因此这类图层视为几何动画，逐帧烘焙；拓扑固定（顶点数不变），通常只产生一个桶。

这解释了此前“海星主体不动”的原因：表达式未被求值，10 个顶点停留在参考帧位置；现在会随空对象与父级 `main` 一起运动。

### 14.3 位置与旋转表达式：沿路径分布、弹性回弹与切线旋转

`lights.json` 的彩灯示例同时用到了三种表达式，任意一种缺失都会让效果失真：

1. **沿路径分布（Trace Path）**：灯泡空对象的位置写成
   ```js
   thisComp.layer('wire').toComp(pathToTrace.pointOnPath(progress))
   ```
   转换器从图层的 `Pseudo/ADBE Trace Path` 效果读取 `progress`（百分比），按 lottie-web `pointOnPath` 的采样语义在目标路径当帧展开的曲线上取点，再用目标层链条矩阵换算到合成坐标（`toComp`）。由于目标路径本身带顶点跟随表达式，电线回弹时灯泡会一并移动。
   注意 lottie-web 的 `_segmentsLength` **只在首次调用时缓存**（源帧 0），此后每帧都用这张首帧长度表把百分比映射到「段索引 + 段内比例」，再在当帧曲线上按对应贝塞尔参数取点；路径变形时若按当帧弧长重算，灯泡会沿电线滑动（实测最大偏差约 13.5px），故转换器在采样起点预先固化一张 `PathLengths` 参考表。
2. **弹性回弹（Position Overshoot）**：电线中点的位置表达式在关键帧之后叠加衰减正弦：
   ```js
   v = velocityAtTime(key(n).time - thisComp.frameDuration / 10)
   value + v / 100 * amp * sin(2π * freq * t) / exp(decay * t)
   ```
   `amp/freq/decay` 分别取自 `Position - Overshoot / Bounce / Friction` 滑块（按 2.5、20、20 归一）。转换器用关键帧曲线在 `keyTime - 0.1 帧` 处做数值微分得到速度，再按上式叠加位移。
3. **切线旋转（Tangent Rotation）**：灯泡图层的旋转写成
   ```js
   var pathTan = pathToTrace.tangentOnPath(progress);
   radiansToDegrees(Math.atan2(pathTan[1], pathTan[0]))
   ```
   转换器复现 `tangentOnPath` 的取法：在 `progress` 与 `progress + 0.1%` 两点（首尾端点另有 0.999 / 闭合标志的特殊处理）求连线方向得到单位切线，经目标层链条矩阵变换到合成空间后取 `atan2` 得角度；若当前图层有父级，再减去父级链条的旋转，得到图层局部 `r`。因此灯泡随电线上下振动同步摆动，角度与电线切线一致（回归实测各帧误差 ≤ 0.7°）。

每帧求值顺序：先算弹性回弹覆盖位置 → 再算沿路径分布与切线旋转（目标路径依赖回弹后的顶点和空对象坐标）→ 最后写世界矩阵。三类表达式都只影响图层位姿（位置 / 旋转），不改变网格拓扑。

---

## 15. 扩展指南

- **新增形状类型**：在 [`shape.rs`](../rust/lottie2mesh/src/shape.rs) 的 `eval_group_items` 中生成 `PathItem`；若只是新几何，无需改 mesh 管线。
- **新增样式类型**：产出新的 `Drawable`；若需要新拓扑（例如虚线），在 [`mesh.rs`](../rust/lottie2mesh/src/mesh.rs) 增加 `DrawKind` 分支与索引生成。
- **新增图层类型**：在 [`compile.rs`](../rust/lottie2mesh/src/compile.rs) 的 `build_content` 增加分支；纯色层与形状层可参考现有实现。
- **修改二进制格式**：同步修改 `output.rs`、`sdk/index.mjs`、`sdk/index.d.mts`，并递增 header 版本与 `l2d_version()`。
- **嵌入其它 Node 项目**：直接复制 `sdk/` 与 wasm 产物，或把 `rust/lottie2mesh` 作为 path 依赖直接调用 `compile_to_bytes`。

---

## 16. 验证方法

1. **CLI 冒烟**：`node sdk/cli.mjs public/animations/starfish.json out.l2d3 --fps=30`，检查输出的帧数、网格数、顶点/三角形统计与转换耗时。
2. **原画对照**：页面左侧 lottie-web、右侧 three.js 同步播放，重点检查旋转方向、父级轨道、形变与透明度。
3. **数值回归**：对已知父级旋转场景抽查子层原点轨迹。例如父级 `0 → 360°`、子层偏移 `(0.72, 0)` 时，原点应依次经过 `(0.72,0) → (0,-0.72) → (-0.72,0) → (0,0.72)`（顺时针），误差应在 `1e-5` 量级。
4. **预算检查**：对高顶点样例观察 `frame_count` 是否被 8M float 预算自动下调，并确认播放仍流畅。
5. **参数回归**：拖动边缘平滑/采样帧率后确认自动重转换、播放进度保持、无控制台报错。
