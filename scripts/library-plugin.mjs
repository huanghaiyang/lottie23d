// Vite 插件：把上传的 Lottie 写进项目根目录的 data/，作为本地测试资源库。
//
// 前端只依赖五个同源接口（无需 CORS）：
//   GET    /api/library            列出 data/ 下的 .json 文件
//   POST   /api/library?name=x.json 请求体为原始 JSON 文本，写入 data/
//   GET    /api/library/file?name=x 读取单个文件原文
//   DELETE /api/library?name=x.json 删除单个文件
//   POST   /api/library/rename      批量改名：{ items: [{ from, to }] }
//
// 这些接口只在 `npm run dev` / `npm run preview` 下可用；纯静态部署时
// 前端会隐藏资源库入口，原有“拖入即预览”的流程不受影响。

import { existsSync } from 'node:fs';
import { mkdir, readdir, readFile, rename, rm, stat, writeFile } from 'node:fs/promises';
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

/** 读取请求体并解析为 JSON 对象。 */
async function readJsonBody(req) {
  const text = await readBody(req);
  try {
    return JSON.parse(text);
  } catch {
    throw new LibraryError('请求体不是合法的 JSON');
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
      if (url.pathname === '/rename' && req.method === 'POST') {
        const body = await readJsonBody(req);
        const items = Array.isArray(body?.items) ? body.items : [];
        if (items.length === 0) {
          throw new LibraryError('没有需要改名的文件');
        }
        if (items.length > 500) {
          throw new LibraryError('单次改名最多 500 个文件');
        }
        const sources = items.map((item) => sanitizeName(item?.from));
        const targets = items.map((item) => sanitizeName(item?.to));
        if (new Set(sources).size !== sources.length) {
          throw new LibraryError('源文件名重复');
        }
        if (new Set(targets).size !== targets.length) {
          throw new LibraryError('目标文件名重复');
        }
        await mkdir(dataDir, { recursive: true });
        for (const name of sources) {
          if (!existsSync(join(dataDir, name))) {
            throw new LibraryError(`文件不存在：${name}`, 404);
          }
        }
        const sourceSet = new Set(sources);
        for (const name of targets) {
          if (existsSync(join(dataDir, name)) && !sourceSet.has(name)) {
            throw new LibraryError(`目标文件已存在：${name}`, 409);
          }
        }

        // 两阶段改名：先把源文件挪到临时名，再落到目标名。
        // 否则 a→b、b→c 这类链式改名会在第一步就撞上还没挪走的 b。
        const stamp = Date.now();
        const temps = sources.map((_, index) => `.rename-${stamp}-${index}.tmp`);
        const moved = [];
        try {
          for (const [index, from] of sources.entries()) {
            await rename(join(dataDir, from), join(dataDir, temps[index]));
            moved.push(index);
          }
        } catch (error) {
          // 中途失败时把已挪走的文件放回原名，避免资源库出现半成品状态。
          for (const index of moved.reverse()) {
            try {
              await rename(join(dataDir, temps[index]), join(dataDir, sources[index]));
            } catch {
              // 回滚失败只能保留临时文件，交由调用方看到原始错误。
            }
          }
          throw error;
        }
        for (const [index, to] of targets.entries()) {
          await rename(join(dataDir, temps[index]), join(dataDir, to));
        }
        sendJson(res, 200, {
          ok: true,
          renamed: sources.map((from, index) => ({ from, to: targets[index] })),
        });
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
