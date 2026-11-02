import * as THREE from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import type { Model, MeshModel } from '../../sdk/index.mjs';

interface Entry {
  data: MeshModel;
  mesh: THREE.Mesh;
  geometry: THREE.BufferGeometry;
  material: THREE.MeshLambertMaterial;
  /** Constant base matrix for statically transformed layers. */
  base: THREE.Matrix4;
  track: Float32Array | null;
  transformStatic: boolean;
  colors: Float32Array | null;
  position: THREE.BufferAttribute;
  positionsAnimated: boolean;
}

/** Tracks that never change only need one matrix upload, not one per frame. */
function isConstantTrack(track: Float32Array, frameCount: number): boolean {
  if (frameCount <= 1) {
    return true;
  }
  for (let frame = 1; frame < frameCount; frame += 1) {
    const offset = frame * 16;
    for (let i = 0; i < 16; i += 1) {
      if (track[offset + i] !== track[i]) {
        return false;
      }
    }
  }
  return true;
}

/**
 * three.js stage for the Rust-compiled model.
 *
 * Every mesh keeps a fixed BufferGeometry; per frame we only copy baked vertex
 * positions (animated shapes) and write the world matrix directly, skipping
 * any parent multiplication or work on layers that never move.
 */
export class Stage {
  private readonly renderer: THREE.WebGLRenderer;
  private readonly scene = new THREE.Scene();
  private readonly camera: THREE.PerspectiveCamera;
  private readonly controls: OrbitControls;
  private readonly host: HTMLElement;
  private readonly entries: Entry[] = [];
  private model: Model | null = null;
  private frameIndex = -1;
  private dirty = true;
  private depthAmount = 1;
  private thicknessAmount = 1;
  private pixelRatio = 1.5;
  private lowFpsWindows = 0;
  private readonly observer: ResizeObserver;

  constructor(host: HTMLElement) {
    this.host = host;
    this.renderer = new THREE.WebGLRenderer({
      antialias: true,
      alpha: true,
      powerPreference: 'high-performance',
    });
    this.pixelRatio = Math.min(window.devicePixelRatio || 1, 1.5);
    this.renderer.setPixelRatio(this.pixelRatio);
    this.renderer.setClearColor(0x000000, 0);
    this.renderer.domElement.className = 'three-canvas';
    host.appendChild(this.renderer.domElement);

    this.camera = new THREE.PerspectiveCamera(40, 1, 0.1, 60);
    this.camera.position.set(0, 1.15, 7.2);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.target.set(0, 0, 0);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.09;
    this.controls.rotateSpeed = 0.85;
    this.controls.minDistance = 2.4;
    this.controls.maxDistance = 18;
    this.controls.autoRotate = true;
    this.controls.autoRotateSpeed = 0.65;
    this.controls.addEventListener('change', () => {
      this.dirty = true;
    });

    const ambient = new THREE.AmbientLight(0xffffff, 1.25);
    const key = new THREE.DirectionalLight(0xdfe9ff, 2.4);
    key.position.set(2.4, 3.2, 4.2);
    const rim = new THREE.DirectionalLight(0xffb98a, 1.6);
    rim.position.set(-3.4, -1.6, -2.2);
    const fill = new THREE.DirectionalLight(0x86a8ff, 0.9);
    fill.position.set(-1.5, -2.8, 3.4);
    this.scene.add(ambient, key, rim, fill);

    this.observer = new ResizeObserver(() => this.resize());
    this.observer.observe(host);
    this.resize();
  }

  get hostElement(): HTMLElement {
    return this.host;
  }

  setModel(model: Model): void {
    this.clear();
    this.model = model;
    this.frameIndex = -1;

    // Meshes of the same layer share a transform track; detect statically
    // transformed layers only once per track.
    const staticTracks = new Map<number, boolean>();

    for (const data of model.meshes) {
      const positionsAnimated = data.animated && data.framePositions != null;
      const positions = positionsAnimated
        ? new Float32Array(data.positions)
        : data.positions;

      const geometry = new THREE.BufferGeometry();
      const position = new THREE.BufferAttribute(positions, 3);
      position.setUsage(positionsAnimated ? THREE.DynamicDrawUsage : THREE.StaticDrawUsage);
      geometry.setAttribute('position', position);
      geometry.setIndex(new THREE.BufferAttribute(data.indices, 1));

      const track = model.transforms[data.transformIndex] ?? null;
      let transformStatic = staticTracks.get(data.transformIndex);
      if (transformStatic === undefined) {
        transformStatic = track == null || isConstantTrack(track, model.frameCount);
        staticTracks.set(data.transformIndex, transformStatic);
      }

      const colors = data.colors;
      const alpha = colors ? colors[3] : data.color[3];
      // Lottie colours are sRGB; convert once here so static and animated
      // colours share the same linear-space representation as three.js.
      const baseColor = new THREE.Color().setRGB(
        data.color[0],
        data.color[1],
        data.color[2],
        THREE.SRGBColorSpace,
      );
      const material = new THREE.MeshLambertMaterial({
        color: baseColor,
        flatShading: true,
        side: THREE.DoubleSide,
        // Opaque constant colours skip blending and per-frame sorting entirely.
        transparent: colors != null || alpha < 0.999,
        opacity: Math.max(0, Math.min(1, alpha)),
      });

      const mesh = new THREE.Mesh(geometry, material);
      mesh.matrixAutoUpdate = false;
      mesh.matrixWorldAutoUpdate = false;
      mesh.matrixWorldNeedsUpdate = false;
      mesh.frustumCulled = false;
      mesh.renderOrder = 0;
      mesh.visible = alpha >= 0.004;
      this.scene.add(mesh);

      const base = new THREE.Matrix4();
      if (track) {
        base.fromArray(track, 0);
      }

      this.entries.push({
        data,
        mesh,
        geometry,
        material,
        base,
        track,
        transformStatic,
        colors,
        position,
        positionsAnimated,
      });
    }

    this.applyFrame(0);
    this.dirty = true;
  }

  setFrame(frame: number): void {
    if (this.model == null) {
      return;
    }
    const clamped = Math.max(0, Math.min(this.model.frameCount - 1, Math.round(frame)));
    if (clamped === this.frameIndex) {
      return;
    }
    this.applyFrame(clamped);
    this.dirty = true;
  }

  setDepthScale(value: number): void {
    this.depthAmount = value;
    if (this.model != null) {
      this.applyFrame(Math.max(0, this.frameIndex));
      this.dirty = true;
    }
  }

  setThicknessScale(value: number): void {
    this.thicknessAmount = value;
    if (this.model != null) {
      this.applyFrame(Math.max(0, this.frameIndex));
      this.dirty = true;
    }
  }

  setAutoRotate(enabled: boolean): void {
    this.controls.autoRotate = enabled;
    this.dirty = true;
  }

  /**
   * Lower the render resolution when playback keeps missing the frame budget.
   * Called from the FPS readout, so it reacts within a few seconds and never
   * costs anything while the scene is smooth.
   */
  adaptQuality(fps: number): void {
    if (fps >= 45 || this.pixelRatio <= 0.8) {
      this.lowFpsWindows = 0;
      return;
    }
    this.lowFpsWindows += 1;
    if (this.lowFpsWindows < 6) {
      return;
    }
    this.lowFpsWindows = 0;
    this.pixelRatio = Math.max(0.75, this.pixelRatio * 0.8);
    this.renderer.setPixelRatio(this.pixelRatio);
    this.resize();
  }

  resetView(): void {
    this.camera.position.set(0, 1.15, 7.2);
    this.controls.target.set(0, 0, 0);
    this.controls.update();
    this.dirty = true;
  }

  render(): void {
    const moved = this.controls.update();
    if (moved || this.dirty) {
      this.renderer.render(this.scene, this.camera);
      this.dirty = false;
    }
  }

  dispose(): void {
    this.observer.disconnect();
    this.clear();
    this.controls.dispose();
    this.renderer.dispose();
    this.renderer.domElement.remove();
  }

  private applyFrame(frame: number): void {
    if (this.model == null) {
      return;
    }
    this.frameIndex = frame;
    const frameCount = this.model.frameCount;
    const trackOffset = Math.min(frame, frameCount - 1) * 16;
    const depthDelta = this.depthAmount - 1;
    const thickness = this.thicknessAmount;

    for (const entry of this.entries) {
      const { data, mesh, material, position, colors, track } = entry;

      // Write the world matrix directly: layers sit under the scene root, so
      // there is no parent multiply to do and nothing to invalidate.
      if (entry.transformStatic) {
        mesh.matrixWorld.copy(entry.base);
      } else if (track != null) {
        mesh.matrixWorld.fromArray(track, trackOffset);
      }
      const elements = mesh.matrixWorld.elements;
      if (depthDelta !== 0) {
        // Depth is baked into the track, so the slider only adds a delta.
        elements[14] += data.depth * depthDelta;
      }
      if (thickness !== 1) {
        // Equivalent to post-multiplying by scale(1, 1, thickness).
        elements[2] *= thickness;
        elements[6] *= thickness;
        elements[10] *= thickness;
      }

      if (colors != null) {
        const colorBase = frame * 4;
        const alpha = colors[colorBase + 3];
        if (alpha < 0.004) {
          mesh.visible = false;
          continue;
        }
        mesh.visible = true;
        material.opacity = alpha;
        material.color.setRGB(
          colors[colorBase],
          colors[colorBase + 1],
          colors[colorBase + 2],
          THREE.SRGBColorSpace,
        );
      }

      if (entry.positionsAnimated && data.framePositions) {
        const count = data.vertexCount * 3;
        const offset = frame * count;
        if (offset + count <= data.framePositions.length) {
          position.array.set(data.framePositions.subarray(offset, offset + count));
          position.needsUpdate = true;
        }
      }
    }
  }

  private clear(): void {
    for (const entry of this.entries) {
      this.scene.remove(entry.mesh);
      entry.geometry.dispose();
      entry.material.dispose();
    }
    this.entries.length = 0;
    this.model = null;
  }

  private resize(): void {
    const width = Math.max(1, this.host.clientWidth);
    const height = Math.max(1, this.host.clientHeight);
    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.dirty = true;
  }
}
