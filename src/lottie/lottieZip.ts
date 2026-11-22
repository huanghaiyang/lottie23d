/**
 * `.lottie` 归档解析：标准 Lottie 是一个 ZIP 包，根目录放 `animation.json`，
 * 内嵌位图放在 `images/` 等子目录。浏览器侧用 fflate 解压，把图片资产
 * 重映射为 blob URL，交给与普通 JSON 相同的播放 / 转换链路。
 */

import { strFromU8, unzipSync } from 'fflate';
import type { LottieData } from './types';

/** 识别 Lottie 归档：标准 `.lottie`，兼容部分工具导出的 `.zip`。 */
export function isLottieArchive(name: string): boolean {
  return /\.(lottie|zip)$/i.test(name);
}

/** 归档内图片资产的最小结构（预合成资产没有 `p` 字段，会被跳过）。 */
interface ArchiveImageAsset {
  id?: string;
  w?: number;
  h?: number;
  u?: string;
  p?: string;
  e?: number;
  [key: string]: unknown;
}

export interface ParsedLottie {
  data: LottieData;
  /** 本次解析创建的 blob URL；切换文档时可按需释放。 */
  objectUrls: string[];
}

/** 动画 JSON 的候选文件名，按优先级匹配。 */
const JSON_CANDIDATES = ['animation.json', 'lottie.json', 'data.json'];

/** dotLottie v1 清单结构（只需动画 id 用于定位包内 JSON）。 */
interface DotLottieManifest {
  animations?: Array<{ id?: string }>;
}

/** 判断条目是否为清单文件本身（不能当作 Lottie 动画解析）。 */
function isManifestEntry(name: string): boolean {
  return normalizeZipPath(name) === 'manifest.json';
}

/**
 * 在解压结果里定位 Lottie 动画 JSON：
 * 1. dotLottie v1：读 `manifest.json`，按 `animations[0].id` 找 `animations/<id>.json`，
 *    缺失时退回 `animations/` 目录下的第一个 JSON；
 * 2. 标准归档：根目录 `animation.json / lottie.json / data.json`；
 * 3. 兜底：包内任意 `.json`（排除 manifest）。
 */
function resolveAnimationJson(files: Record<string, Uint8Array>): string | undefined {
  const manifestEntry = files['manifest.json'];
  if (manifestEntry != null) {
    try {
      const manifest = JSON.parse(strFromU8(manifestEntry)) as DotLottieManifest;
      const firstId = manifest.animations?.[0]?.id;
      if (firstId != null) {
        const byId = `animations/${firstId}.json`;
        if (files[byId] != null) {
          return byId;
        }
      }
      const inAnimations = Object.keys(files).find(
        (name) =>
          normalizeZipPath(name).startsWith('animations/') &&
          name.toLowerCase().endsWith('.json'),
      );
      if (inAnimations != null) {
        return inAnimations;
      }
    } catch {
      // 清单损坏时继续走通用候选，不阻断解析。
    }
  }

  const standard = JSON_CANDIDATES.find((candidate) => files[candidate] != null);
  if (standard != null) {
    return standard;
  }
  return Object.keys(files).find(
    (name) => name.toLowerCase().endsWith('.json') && !isManifestEntry(name),
  );
}

/** 归一化 ZIP 内路径：反斜杠转正斜杠，去掉 `./` 与前导 `/`。 */
function normalizeZipPath(path: string): string {
  return path.replace(/\\/g, '/').replace(/^\.\//, '').replace(/^\/+/, '');
}

/** 在解压结果里按相对路径查找条目，缺失目录前缀时按文件名兜底匹配。 */
function lookupEntry(
  files: Record<string, Uint8Array>,
  rel: string,
): Uint8Array | undefined {
  const want = normalizeZipPath(rel);
  const direct = files[want];
  if (direct) {
    return direct;
  }
  const base = want.split('/').pop() ?? want;
  return Object.entries(files).find(
    ([name, bytes]) => normalizeZipPath(name).split('/').pop() === base && bytes.length > 0,
  )?.[1];
}

/** 按扩展名推断图片 MIME，兜底为二进制流。 */
function mimeFromName(name: string): string {
  const ext = name.split('.').pop()?.toLowerCase();
  switch (ext) {
    case 'png':
      return 'image/png';
    case 'jpg':
    case 'jpeg':
      return 'image/jpeg';
    case 'webp':
      return 'image/webp';
    case 'gif':
      return 'image/gif';
    case 'svg':
      return 'image/svg+xml';
    default:
      return 'application/octet-stream';
  }
}

/**
 * 解压并解析一个 `.lottie` / `.zip` 文件。
 *
 * 图片资产的 `u + p` 指向包内相对路径；命中条目时把 `p` 替换为 blob URL、
 * 清空 `u`，这样 Rust 侧与 lottie-web 都直接使用该 URL，不再回源到包外路径。
 */
export async function parseLottieArchive(file: Blob): Promise<ParsedLottie> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  let files: Record<string, Uint8Array>;
  try {
    files = unzipSync(bytes);
  } catch (error) {
    throw new Error(
      `无法解压 Lottie 归档：${error instanceof Error ? error.message : String(error)}`,
    );
  }

  const jsonName = resolveAnimationJson(files);
  if (!jsonName) {
    throw new Error('归档内未找到 animation.json');
  }

  let data: LottieData;
  try {
    data = JSON.parse(strFromU8(files[jsonName])) as LottieData;
  } catch (error) {
    throw new Error(
      `animation.json 解析失败：${error instanceof Error ? error.message : String(error)}`,
    );
  }
  if (data == null || !Array.isArray(data.layers)) {
    throw new Error('归档内的动画缺少 layers 数组');
  }

  const objectUrls: string[] = [];
  const assets = Array.isArray(data.assets) ? data.assets : [];
  for (const raw of assets) {
    const asset = raw as ArchiveImageAsset;
    if (asset.p == null || typeof asset.p !== 'string') {
      continue;
    }
    // 内嵌 base64（e=1）或已是 data URI 的资产直接保留。
    if (asset.e === 1 || asset.p.startsWith('data:')) {
      continue;
    }
    const rel = normalizeZipPath(`${asset.u ?? ''}${asset.p}`);
    const entry = lookupEntry(files, rel);
    if (!entry) {
      continue;
    }
    const entryCopy = new Uint8Array(entry.length);
    entryCopy.set(entry);
    const blob = new Blob([entryCopy], { type: mimeFromName(asset.p) });
    const url = URL.createObjectURL(blob);
    asset.u = '';
    asset.p = url;
    objectUrls.push(url);
  }

  return { data, objectUrls };
}
