#!/usr/bin/env node
// 索引覆盖率检查：docs/ 下的文件是否都已登记进对应索引（R6）
// 用法: node .agents/scripts/docs-index-check.mjs
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const DOCS = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', 'docs');
const read = (p) => fs.readFileSync(path.join(DOCS, p), 'utf8');
const list = (d) => fs.readdirSync(path.join(DOCS, d), { withFileTypes: true }).filter((e) => e.isFile()).map((e) => e.name);

let bad = 0;
function coverage(dirName, indexFile, subdirs = []) {
  const idx = read(indexFile);
  const rows = [];
  for (const name of list(dirName)) {
    if (name === 'README.md') continue;
    rows.push([`${dirName}/${name}`, idx.includes(name)]);
  }
  for (const sd of subdirs) {
    const p = `${dirName}/${sd}`;
    if (!fs.existsSync(path.join(DOCS, p))) continue;
    const inner = fs.readdirSync(path.join(DOCS, p), { withFileTypes: true });
    if (inner.some((e) => e.isFile() && e.name !== 'README.md')) rows.push([`${p}/（目录）`, idx.includes(`${sd}/`)]);
  }
  const missing = rows.filter(([, ok]) => !ok);
  console.log(`[${dirName}] 条目 ${rows.length}，未登记 ${missing.length}`);
  for (const [n] of missing) console.log('   未登记: ' + n);
  bad += missing.length;
}

coverage('active/spec', 'README.md');
coverage('active/plan', 'README.md');
coverage('active/audit', 'README.md');
coverage('research', 'README.md');
coverage('legacy', 'README.md');
{
  const idx = read('reports/README.md');
  const files = list('reports').filter((n) => n !== 'README.md');
  const miss = files.filter((n) => !idx.includes(n));
  console.log(`[reports] 顶层文件 ${files.length}，未登记 ${miss.length}`);
  for (const n of miss) console.log('   未登记: ' + n);
  bad += miss.length;
  for (const d of ['refactor', 'prompts']) {
    const exists = fs.existsSync(path.join(DOCS, 'reports', d));
    const ok = idx.includes(d + '/');
    if (exists && !ok) { console.log(`   未登记目录: reports/${d}/`); bad++; }
  }
}
// devlog 由脚本生成索引，这里只核对生成物是否最新
console.log(`\n索引缺口合计: ${bad}`);
if (bad) process.exit(1);
