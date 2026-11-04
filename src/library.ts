/**
 * 资源库客户端：浏览器上传的 Lottie 由开发服务器写入项目根目录的 `data/`。
 *
 * 接口只在 `npm run dev` / `npm run preview` 下存在；静态部署时第一次
 * `listLibrary()` 会失败，调用方据此隐藏资源库入口。
 */

import type { LottieData } from './lottie/types';

/** 资源库中的一个文件；时间戳来自文件系统，单位毫秒。 */
export interface LibraryFile {
  name: string;
  size: number;
  modified: number;
}

/** 用 baseURI 拼绝对地址，兼容部署在子目录下的情况。 */
function endpoint(path = ''): string {
  return new URL(`api/library${path}`, document.baseURI).toString();
}

/** 统一请求封装：非 2xx 或 `ok: false` 都转成 Error，便于直接提示。 */
async function request(url: string, init?: RequestInit): Promise<Record<string, unknown>> {
  const response = await fetch(url, init);
  let payload: Record<string, unknown> | null = null;
  try {
    payload = (await response.json()) as Record<string, unknown>;
  } catch {
    payload = null;
  }
  if (!response.ok || payload == null || payload.ok === false) {
    const message = typeof payload?.error === 'string' ? payload.error : `HTTP ${response.status}`;
    throw new Error(message);
  }
  return payload;
}

/** 列出 data/ 下的全部 Lottie 文件（按名称自然排序）。 */
export async function listLibrary(): Promise<LibraryFile[]> {
  const payload = await request(endpoint());
  return (payload.files as LibraryFile[]) ?? [];
}

/** 把一份 Lottie 原文写入资源库；同名文件会被覆盖。 */
export async function saveLibraryFile(name: string, text: string): Promise<void> {
  await request(endpoint(`?name=${encodeURIComponent(name)}`), {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: text,
  });
}

/** 读取资源库中的单个文件并解析为 Lottie 数据。 */
export async function readLibraryFile(name: string): Promise<LottieData> {
  const response = await fetch(endpoint(`/file?name=${encodeURIComponent(name)}`), {
    cache: 'no-store',
  });
  if (!response.ok) {
    throw new Error(`无法读取 ${name}（HTTP ${response.status}）`);
  }
  return (await response.json()) as LottieData;
}

/** 从资源库删除文件。 */
export async function deleteLibraryFile(name: string): Promise<void> {
  await request(endpoint(`?name=${encodeURIComponent(name)}`), { method: 'DELETE' });
}
