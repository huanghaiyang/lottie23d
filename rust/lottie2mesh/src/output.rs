//! 二进制模型编码器（格式版本 2）。
//!
//! 所有偏移均为从 blob 起始处算起的**字节偏移**，整数与浮点一律小端。
//!
//! ```text
//! header（96 字节）
//! mesh records（每个 96 字节）
//! transform tracks（每条 frame_count * 16 个 f32）
//! mesh 数据块：静态顶点、逐帧顶点、各桶 indices、颜色轨迹、
//!               桶表、逐帧顶点偏移、逐帧桶 id、名称
//! ```
//!
//! 播放端直接用这些缓冲创建 TypedArray，运行时无需 JSON 反序列化。
//! 数据块按“先记偏移、再追加数据”的顺序写入，因此各网格的数据块可以
//! 交错排布，偏移字段在数据落盘后才回填。

use crate::compile::Compiled;

/// header 固定字节数。
const HEADER_SIZE: usize = 96;
/// 单个 mesh record 固定字节数（字段偏移见 [`encode`]）。
const RECORD_SIZE: usize = 96;
/// 当前格式版本（`l2d_version` 返回同一值）。
const VERSION: u32 = 2;

/// 把编译结果序列化为 `.l2d3` 二进制。
///
/// 先写 header 与 records 组成的元信息区，再逐网格追加数据块；最终总长度
/// 回填到 header 的偏移 52。返回完整的二进制缓冲。
pub fn encode(model: &Compiled) -> Vec<u8> {
    let record_start = HEADER_SIZE;
    // records 紧接 header，transform tracks 紧接 records（每条 16 f32/帧）。
    let transform_start = record_start + RECORD_SIZE * model.meshes.len();
    let transform_bytes = model.transforms.len() * model.frame_count * 16 * 4;
    // 元信息区 = header + records + transforms；数据块从其后开始。
    let meta_size = transform_start + transform_bytes;

    let mut blob = vec![0u8; meta_size];
    let mut data: Vec<u8> = Vec::new();

    // header 字段偏移（字节）：
    //   0 magic 0x4C323344（"L23D"）、4 版本、8 宽、12 高
    //   16 帧率、20 ip、24 op、28 采样 fps
    //   32 帧数、36 网格数、40 变换轨迹数、44 总顶点数、48 总索引数
    //   52 二进制总字节数、56 转换耗时(µs)、64 名称偏移、68 名称长度
    put_u32(&mut blob, 0, 0x4C32_3344);
    put_u32(&mut blob, 4, VERSION);
    put_f32(&mut blob, 8, model.width);
    put_f32(&mut blob, 12, model.height);
    put_f32(&mut blob, 16, model.frame_rate);
    put_f32(&mut blob, 20, model.ip);
    put_f32(&mut blob, 24, model.op);
    put_f32(&mut blob, 28, model.sample_fps);
    put_u32(&mut blob, 32, model.frame_count as u32);
    put_u32(&mut blob, 36, model.meshes.len() as u32);
    put_u32(&mut blob, 40, model.transforms.len() as u32);
    put_u32(&mut blob, 56, model.conversion_us);

    let total_vertices: u32 = model
        .meshes
        .iter()
        .map(|mesh| mesh.positions.len() as u32 / 3)
        .sum();
    let total_indices: u32 = model
        .meshes
        .iter()
        .map(|mesh| mesh.buckets.iter().map(|bucket| bucket.indices.len() as u32).sum::<u32>())
        .sum();
    put_u32(&mut blob, 44, total_vertices);
    put_u32(&mut blob, 48, total_indices);

    // 变换轨迹：每条按帧连续存放 16 个 f32（列主序 4×4 世界矩阵）。
    for (track_index, track) in model.transforms.iter().enumerate() {
        let base = transform_start + track_index * model.frame_count * 16 * 4;
        for (i, value) in track.iter().enumerate() {
            blob[base + i * 4..base + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    // 逐网格写 record（元信息）并在 `data` 中追加其数据块。
    for (index, mesh) in model.meshes.iter().enumerate() {
        let record = record_start + index * RECORD_SIZE;
        // record 字段偏移（字节）：
        //   0 kind、4 flags、8..24 静态颜色 RGBA、24 深度、28 半厚度
        //   32 顶点数、36 索引长度、40 静态顶点偏移、44 逐帧顶点偏移
        //   48 基础 indices 偏移、52 颜色轨迹偏移、56 变换轨迹下标
        //   60 渲染顺序、64 采样点数、68 名称偏移、72 名称长度
        //   76 桶数量、80 桶表偏移、84 逐帧偏移表、88 逐帧桶 id
        put_u32(&mut blob, record, mesh.kind as u32);
        // flags 位：bit0 有逐帧顶点、bit1 有颜色轨迹、
        // bit2 有 frame_offsets、bit3 使用多桶（>1 套 indices）。
        let mut flags = if mesh.frame_positions.is_some() { 1 } else { 0 };
        if !mesh.color_track.is_empty() {
            flags |= 2;
        }
        if !mesh.frame_offsets.is_empty() {
            flags |= 4;
        }
        let bucketized = mesh.buckets.len() > 1;
        if bucketized {
            flags |= 8;
        }
        put_u32(&mut blob, record + 4, flags);
        put_f32(&mut blob, record + 8, mesh.static_color[0]);
        put_f32(&mut blob, record + 12, mesh.static_color[1]);
        put_f32(&mut blob, record + 16, mesh.static_color[2]);
        put_f32(&mut blob, record + 20, mesh.static_color[3]);
        put_f32(&mut blob, record + 24, mesh.depth);
        put_f32(&mut blob, record + 28, mesh.half_thickness);
        // 顶点数取所有桶与初始顶点缓冲中的最大值，保证任一桶都能写入。
        let vertex_count: u32 = mesh
            .buckets
            .iter()
            .map(|bucket| bucket.vertex_count)
            .max()
            .unwrap_or(mesh.positions.len() as u32 / 3)
            .max(mesh.positions.len() as u32 / 3);
        put_u32(&mut blob, record + 32, vertex_count);
        put_u32(&mut blob, record + 36, mesh.indices.len() as u32);
        put_u32(&mut blob, record + 56, mesh.transform_index as u32);
        put_u32(&mut blob, record + 60, mesh.render_order);
        put_u32(&mut blob, record + 64, mesh.point_count);
        put_u32(&mut blob, record + 76, mesh.buckets.len() as u32);

        // 静态参考顶点 `[x, y, z]`，每 3 个 f32 一个顶点。
        let positions_offset = (meta_size + data.len()) as u32;
        for value in &mesh.positions {
            data.extend_from_slice(&value.to_le_bytes());
        }
        put_u32(&mut blob, record + 40, positions_offset);

        // 逐帧顶点连续打包；读取时以 frame_offsets 的 float 下标切片。
        let frame_positions_offset = if let Some(frames) = &mesh.frame_positions {
            let offset = (meta_size + data.len()) as u32;
            for value in frames {
                data.extend_from_slice(&value.to_le_bytes());
            }
            offset
        } else {
            0
        };
        put_u32(&mut blob, record + 44, frame_positions_offset);

        // 每个拓扑桶一套 indices；先记录偏移与规模，稍后写桶表。
        let mut bucket_entries: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(mesh.buckets.len());
        for bucket in &mesh.buckets {
            let indices_offset = (meta_size + data.len()) as u32;
            for value in &bucket.indices {
                data.extend_from_slice(&value.to_le_bytes());
            }
            bucket_entries.push((
                indices_offset,
                bucket.indices.len() as u32,
                bucket.vertex_count,
                bucket.point_count,
            ));
        }
        // 基础 indices 指向第一个桶，兼容只读取一套索引的播放路径。
        let base_indices_offset = bucket_entries
            .first()
            .map(|entry| entry.0)
            .unwrap_or(0);
        put_u32(&mut blob, record + 48, base_indices_offset);

        let color_offset = if mesh.color_track.is_empty() {
            0
        } else {
            let offset = (meta_size + data.len()) as u32;
            for value in &mesh.color_track {
                data.extend_from_slice(&value.to_le_bytes());
            }
            offset
        };
        put_u32(&mut blob, record + 52, color_offset);

        if bucketized {
            // 仅在多桶时写桶表：每项 16 字节
            // [indices 偏移, 索引数, 顶点数, 采样点数]。
            let table_offset = (meta_size + data.len()) as u32;
            for entry in &bucket_entries {
                data.extend_from_slice(&entry.0.to_le_bytes());
                data.extend_from_slice(&entry.1.to_le_bytes());
                data.extend_from_slice(&entry.2.to_le_bytes());
                data.extend_from_slice(&entry.3.to_le_bytes());
            }
            put_u32(&mut blob, record + 80, table_offset);
        }

        if !mesh.frame_offsets.is_empty() {
            // 逐帧顶点起点表：u32 的 float 下标，长度 frame_count + 1。
            let offset = (meta_size + data.len()) as u32;
            for value in &mesh.frame_offsets {
                data.extend_from_slice(&value.to_le_bytes());
            }
            put_u32(&mut blob, record + 84, offset);
        }

        if bucketized && !mesh.frame_buckets.is_empty() {
            // 逐帧桶 id（u8），仅多桶时写入。
            let offset = (meta_size + data.len()) as u32;
            data.extend_from_slice(&mesh.frame_buckets);
            put_u32(&mut blob, record + 88, offset);
        }

        // 名称按 4 字节对齐存放：不足部分补 0，偏移 68/72 记录起点与长度。
        let name_offset = (meta_size + data.len()) as u32;
        let name_bytes = mesh.name.as_bytes();
        data.extend_from_slice(name_bytes);
        let padding = (4 - (name_bytes.len() % 4)) % 4;
        data.extend(std::iter::repeat(0u8).take(padding));
        put_u32(&mut blob, record + 68, name_offset);
        put_u32(&mut blob, record + 72, name_bytes.len() as u32);
    }

    blob.extend_from_slice(&data);
    // 先写入不含模型名称的总长度，追加名称后再更新一次。
    let total = blob.len() as u32;
    put_u32(&mut blob, 52, total);

    // 模型名称同样按 4 字节对齐追加，偏移写入 header 的 64/68。
    let name_bytes = model.name.as_bytes();
    let name_offset = blob.len() as u32;
    blob.extend_from_slice(name_bytes);
    let padding = (4 - (name_bytes.len() % 4)) % 4;
    blob.extend(std::iter::repeat(0u8).take(padding));
    put_u32(&mut blob, 64, name_offset);
    put_u32(&mut blob, 68, name_bytes.len() as u32);

    // 追加名称后更新 header 的总字节数。
    let total = blob.len() as u32;
    put_u32(&mut blob, 52, total);
    blob
}

/// 以小端 u32 写入指定字节偏移。
fn put_u32(blob: &mut [u8], offset: usize, value: u32) {
    blob[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

/// 以小端 f32 写入指定字节偏移。
fn put_f32(blob: &mut [u8], offset: usize, value: f32) {
    blob[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
