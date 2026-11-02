export interface ConvertOptions {
  /** Sampling frame rate for baked tracks. Defaults to the document rate. */
  fps?: number;
  /** Hard cap on baked frames (default 600). */
  max_frames?: number;
  /** Extrusion half-thickness in Lottie pixels. */
  thickness?: number;
  /** Edge smoothing radius in Lottie pixels (fills only, clamped to thickness). */
  edge_radius?: number;
  /** camelCase alias of `edge_radius`. */
  edgeRadius?: number;
}

export interface MeshBucket {
  /** Cached triangle indices for this topology. */
  indices: Uint32Array;
  indexCount: number;
  vertexCount: number;
  pointCount: number;
}

export interface MeshModel {
  index: number;
  /** 0 = fill, 1 = stroke. */
  kind: number;
  /** Whether this mesh carries per-frame vertex positions. */
  animated: boolean;
  /** RGBA at the reference frame (0..1). */
  color: Float32Array;
  /** Layer depth in normalised [-0.65, 0.65] space. */
  depth: number;
  halfThickness: number;
  /** Largest vertex count across all topology buckets. */
  vertexCount: number;
  indexCount: number;
  pointCount: number;
  /** Static vertex positions, xyz interleaved. */
  positions: Float32Array;
  /** Baked positions packed frame by frame (see `frameOffsets`), or null. */
  framePositions: Float32Array | null;
  /** Start of every frame in floats (frameCount + 1 entries), or null. */
  frameOffsets: Uint32Array | null;
  /** Topology bucket index per frame, or null when there is a single bucket. */
  frameBuckets: Uint8Array | null;
  /** Cached index sets per topology; always at least one entry. */
  buckets: MeshBucket[];
  /** Indices of the first bucket (kept for simple consumers). */
  indices: Uint32Array;
  /** RGBA per frame, frameCount × 4, or null when the colour is constant. */
  colors: Float32Array | null;
  transformIndex: number;
  renderOrder: number;
  name: string;
}

export interface NodeModel {
  /** Index in render order; also the index into `meshes`. */
  id: number;
  /** Layer name plus drawable name, e.g. "Dot A · 光晕". */
  name: string;
  /** 0 = fill, 1 = stroke. */
  kind: number;
  animated: boolean;
  vertexCount: number;
  indexCount: number;
  renderOrder: number;
}

export interface Model {
  version: number;
  width: number;
  height: number;
  frameRate: number;
  ip: number;
  op: number;
  sampleFps: number;
  frameCount: number;
  meshCount: number;
  transformCount: number;
  totalVertices: number;
  totalIndices: number;
  totalSize: number;
  conversionUs: number;
  name: string;
  duration: number;
  /** Column-major 4×4 world matrices per frame. */
  transforms: Float32Array[];
  meshes: MeshModel[];
  /** Flat node list in render order (id indexes into `meshes`). */
  nodes: NodeModel[];
}

export interface Converter {
  convert(lottie: string | object, options?: ConvertOptions): Model;
  convertBytes(lottie: string | object, options?: ConvertOptions): Uint8Array;
  parseModel(bytes: Uint8Array | ArrayBuffer): Model;
  version: number;
}

export function loadConverter(
  source?: string | URL | ArrayBuffer | ArrayBufferView | WebAssembly.Module,
): Promise<Converter>;

export function parseModel(bytes: Uint8Array | ArrayBuffer): Model;

declare const _default: typeof loadConverter;
export default _default;
