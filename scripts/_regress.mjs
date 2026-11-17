import { readdirSync, readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { loadConverter, parseModel } from '../sdk/index.mjs';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const conv = await loadConverter();
const dirs = ['public/animations', 'data'];
const files = [];
for (const d of dirs) {
  for (const f of readdirSync(join(root, d))) {
    if (f.endsWith('.json')) files.push(join(root, d, f));
  }
}
for (const f of files) {
  try {
    const j = JSON.parse(readFileSync(f, 'utf8'));
    const bytes = conv.convertBytes(JSON.stringify(j), {});
    const m = parseModel(bytes);
    const grad = m.meshes.filter((x) => x.gradient).length;
    console.log(
      f.replace(root + '\\', '').replace(root + '/', '').padEnd(28),
      `meshes=${m.meshCount} grad=${grad} vert=${m.totalVertices} tri=${Math.round(m.totalIndices / 3)} frames=${m.frameCount} kb=${(bytes.byteLength / 1024).toFixed(0)}`,
    );
  } catch (e) {
    console.log(f.padEnd(28), 'ERROR', String(e.message || e).slice(0, 200));
  }
}
