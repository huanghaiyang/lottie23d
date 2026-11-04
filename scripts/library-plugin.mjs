// Vite 插件：把上传的 Lottie 写进项目根目录的 data/，作为本地测试资源库。
//
// 前端只依赖四个同源接口（无需 CORS）：
//   GET    /api/library            列出 data/ 下的 .json 文件
//   POST   /api/library?name=x.json 请求体为原始 JSON 文本，写入 data/
//   GET    /api/library/file?name=x 读取单个文件原文
//   DELETE /api/library?name=x.json 删除单个文件
//
// 这些接口只在 `npm run dev` / `npm run preview` 下可用；纯静态部署时
// 前端会隐藏资源库入口，原有“拖入即预览”的流程不受影响。

import { existsSync } from 'node:fs';
import { mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** 项目根目录（本文件位于 scripts/ 下）。 */
const root = fileURLToPath(new URL('..', import.meta.url));
/** 资源库目录：<项目>/data。 */
export const dataDir = join(root, 'data');
/** URL 前缀，前端所有资源库请求都走这里。 */
const PREFIX = '/api/library';
/** 单文件体积上限：64 MB，足够容纳常规 Bodymovin 导出。 */
const MAX_BODY_BYTES = 64 * 1024 * 1024;

/** 带 HTTP 状态码的接口错误，交给统一错误响应使用。 */
class LibraryError extends Error {
  constructor(message, status = 400) {
    super(message);
    this.status = status;
  }
}

/** 统一输出 JSON 响应；资源库内容随改随变，禁用缓存。 */
function sendJson(res, status, payload) {
  res.statusCode = status;
  res.setHeader('Content-Type', 'application/json; charset=utf-8');
  res.setHeader('Cache-Control', 'no-store');
  res.end(JSON.stringify(payload));
}

/** 读取请求体（UTF-8 文本），超限时直接拒绝。 */
function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    req.on('data', (chunk) => {
      size += chunk.length;
      if (size > MAX_BODY_BYTES) {
        reject(new LibraryError('文件超过 64 MB 上限', 413));
        req.destroy();
        return;
      }
      chunks.push(chunk);
    });
    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
    req.on('error', reject);
  });
}

/**
 * 校验并规范化上传文件名：只接受 data/ 下的单个 .json 文件名，
 * 避免通过 `../` 或绝对路径写到资源库之外。
 */
function sanitizeName(raw) {
  if (!raw) {
    throw new LibraryError('缺少文件名');
  }
  const name = decodeURIComponent(raw).trim();
  if (name.includes('/') || name.includes('\\')) {
    throw new LibraryError('文件名不能包含路径');
  }
  if (!/\.json$/i.test(name) || name.length <= 5) {
    throw new LibraryError('只支持 .json 文件');
  }
  if (name.length > 200 || /[\u0000-\u001f<>:"|?*]/.test(name)) {
    throw new LibraryError('文件名包含非法字符');
  }
  return name;
}

/** 上传内容必须是 Bodymovin 导出的动画（至少含 layers 数组）。 */
function validateLottie(text) {
  let doc;
  try {
    doc = JSON.parse(text);
  } catch {
    throw new LibraryError('不是合法的 JSON 文件');
  }
  if (doc == null || typeof doc !== 'object' || !Array.isArray(doc.layers)) {
    throw new LibraryError('不是 Lottie 动画（缺少 layers 数组）');
  }
}

/** 列出资源库文件；按名称自然排序，便于按批次顺序逐个查看。 */
async function listFiles() {
  await mkdir(dataDir, { recursive: true });
  const entries = await readdir(dataDir, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (!entry.isFile() || !entry.name.toLowerCase().endsWith('.json')) {
      continue;
    }
    const info = await stat(join(dataDir, entry.name));
    files.push({ name: entry.name, size: info.size, modified: info.mtimeMs });
  }
  files.sort((a, b) => a.name.localeCompare(b.name, 'zh-CN', { numeric: true }));
  return files;
}

/** 资源库中间件；挂载在 `/api/library` 下，未匹配的请求继续交给 Vite。 */
export function createLibraryMiddleware() {
  return async function libraryMiddleware(req, res, next) {
    const url = new URL(req.url ?? '/', 'http://localhost');
    try {
      // 列表与写入共用根路径，用请求方法区分。
      if (url.pathname === '/' && req.method === 'GET') {
        sendJson(res, 200, { ok: true, directory: 'data', files: await listFiles() });
        return;
      }
      if (url.pathname === '/' && req.method === 'POST') {
        const name = sanitizeName(url.searchParams.get('name'));
        const text = await readBody(req);
        validateLottie(text);
        await mkdir(dataDir, { recursive: true });
        const target = join(dataDir, name);
        const replaced = existsSync(target);
        await writeFile(target, text, 'utf8');
        const info = await stat(target);
        sendJson(res, 200, {
          ok: true,
          replaced,
          file: { name, size: info.size, modified: info.mtimeMs },
        });
        return;
      }
      if (url.pathname === '/' && req.method === 'DELETE') {
        const name = sanitizeName(url.searchParams.get('name'));
        const target = join(dataDir, name);
        if (!existsSync(target)) {
          throw new LibraryError('文件不存在', 404);
        }
        await rm(target);
        sendJson(res, 200, { ok: true, name });
        return;
      }
      if (url.pathname === '/file' && req.method === 'GET') {
        const name = sanitizeName(url.searchParams.get('name'));
        const target = join(dataDir, name);
        if (!existsSync(target)) {
          throw new LibraryError('文件不存在', 404);
        }
        res.statusCode = 200;
        res.setHeader('Content-Type', 'application/json; charset=utf-8');
        res.setHeader('Cache-Control', 'no-store');
        res.end(await readFile(target, 'utf8'));
        return;
      }
      next();
    } catch (error) {
      const status = error instanceof LibraryError ? error.status : 500;
      sendJson(res, status, {
        ok: false,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  };
}

/** Vite 插件入口：dev 与 preview 都挂同一套中间件。 */
export default function lottieLibrary() {
  const attach = (server) => {
    server.middlewares.use(PREFIX, createLibraryMiddleware());
  };
  return {
    name: 'lottie-library',
    configureServer: attach,
    configurePreviewServer: attach,
  };
}
