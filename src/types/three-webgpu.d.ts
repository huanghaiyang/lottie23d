/**
 * three 0.170 的 `three/webgpu` 子路径在 @types/three 中没有类型声明，
 * 这里只为 Stage 实际用到的成员补一份结构化类型（运行时由 Vite 解析到
 * build/three.webgpu.js）。WebGPURenderer 的公共 API 与 WebGLRenderer 对齐。
 */
declare module 'three/webgpu' {
  import type * as THREE from 'three';

  export class WebGPURenderer {
    readonly domElement: HTMLCanvasElement;
    autoClear: boolean;

    constructor(parameters?: {
      antialias?: boolean;
      alpha?: boolean;
      powerPreference?: string;
      forceWebGL?: boolean;
    });

    setPixelRatio(pixelRatio: number): void;
    setSize(width: number, height: number, updateStyle?: boolean): void;
    setClearColor(color: number | THREE.Color, alpha: number): void;
    setRenderTarget(target: unknown): void;
    setViewport(x: number, y: number, width: number, height: number): void;
    setScissor(x: number, y: number, width: number, height: number): void;
    setScissorTest(enabled: boolean): void;
    clear(depth?: boolean, color?: boolean, stencil?: boolean): void;
    clearDepth(): void;
    render(scene: THREE.Scene, camera: THREE.Camera): void;
    dispose(): void;
  }
}
