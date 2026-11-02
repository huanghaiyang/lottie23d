//! Binary model encoder.
//!
//! Layout (all offsets are byte offsets from the start of the blob):
//!
//! ```text
//! header (96 bytes)
//! mesh records (96 bytes each)
//! transform tracks (frame_count * 16 f32 each)
//! mesh data chunks: static positions, frame positions, per-bucket indices,
//!                   colours, bucket table, frame vertex offsets, frame bucket
//!                   ids, names
//! ```
//!
//! Typed arrays are created directly from these buffers in the JavaScript
//! player, so no JSON serialization is needed at runtime.

use crate::compile::Compiled;

const HEADER_SIZE: usize = 96;
const RECORD_SIZE: usize = 96;
/// Current format version (`l2d_version` mirrors it).
const VERSION: u32 = 2;

pub fn encode(model: &Compiled) -> Vec<u8> {
    let record_start = HEADER_SIZE;
    let transform_start = record_start + RECORD_SIZE * model.meshes.len();
    let transform_bytes = model.transforms.len() * model.frame_count * 16 * 4;
    let meta_size = transform_start + transform_bytes;

    let mut blob = vec![0u8; meta_size];
    let mut data: Vec<u8> = Vec::new();

    // Header.
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

    // Transform tracks.
    for (track_index, track) in model.transforms.iter().enumerate() {
        let base = transform_start + track_index * model.frame_count * 16 * 4;
        for (i, value) in track.iter().enumerate() {
            blob[base + i * 4..base + i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    // Mesh records + data chunks.
    for (index, mesh) in model.meshes.iter().enumerate() {
        let record = record_start + index * RECORD_SIZE;
        put_u32(&mut blob, record, mesh.kind as u32);
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

        let positions_offset = (meta_size + data.len()) as u32;
        for value in &mesh.positions {
            data.extend_from_slice(&value.to_le_bytes());
        }
        put_u32(&mut blob, record + 40, positions_offset);

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

        // One index set per topology bucket.
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
            let offset = (meta_size + data.len()) as u32;
            for value in &mesh.frame_offsets {
                data.extend_from_slice(&value.to_le_bytes());
            }
            put_u32(&mut blob, record + 84, offset);
        }

        if bucketized && !mesh.frame_buckets.is_empty() {
            let offset = (meta_size + data.len()) as u32;
            data.extend_from_slice(&mesh.frame_buckets);
            put_u32(&mut blob, record + 88, offset);
        }

        let name_offset = (meta_size + data.len()) as u32;
        let name_bytes = mesh.name.as_bytes();
        data.extend_from_slice(name_bytes);
        let padding = (4 - (name_bytes.len() % 4)) % 4;
        data.extend(std::iter::repeat(0u8).take(padding));
        put_u32(&mut blob, record + 68, name_offset);
        put_u32(&mut blob, record + 72, name_bytes.len() as u32);
    }

    blob.extend_from_slice(&data);
    let total = blob.len() as u32;
    put_u32(&mut blob, 52, total);

    let name_bytes = model.name.as_bytes();
    let name_offset = blob.len() as u32;
    blob.extend_from_slice(name_bytes);
    let padding = (4 - (name_bytes.len() % 4)) % 4;
    blob.extend(std::iter::repeat(0u8).take(padding));
    put_u32(&mut blob, 64, name_offset);
    put_u32(&mut blob, 68, name_bytes.len() as u32);

    let total = blob.len() as u32;
    put_u32(&mut blob, 52, total);
    blob
}

fn put_u32(blob: &mut [u8], offset: usize, value: u32) {
    blob[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_f32(blob: &mut [u8], offset: usize, value: f32) {
    blob[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
