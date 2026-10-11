#!/usr/bin/env node
// 文档相对链接完整性检查：.agents/** + docs/** + 仓库根 *.md
// 用法: node .agents/scripts/doc-links.mjs
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');
const SKIP_DIR = /(\\|\/)(target|\.git|\.worktrees|node_modules|\.jj|\.reasonix)(\\|\/)/;
const LINK = /\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g;

function walk(dir, filter, out = []) {
  let es; try { es = fs.readdirSync(dir, { withFileTypes: true }); } catch { return out; }
  for (const e of es) {
    const full = path.join(dir, e.name);
    if (SKIP_DIR.test(full + path.sep)) continue;
    if (e.isDirectory()) walk(full, filter, out);
    else if (e.isFile() && filter(e.name)) out.push(full);
  }
  return out;
}

const files = [
  ...walk(path.join(ROOT, '.agents'), (n) => n.endsWith('.md')),
  ...walk(path.join(ROOT, 'docs'), (n) => n.endsWith('.md')),
  ...fs.readdirSync(ROOT).filter((n) => n.endsWith('.md')).map((n) => path.join(ROOT, n)),
];

const broken = [];
let checked = 0;
for (const f of files) {
  for (const m of fs.readFileSync(f, 'utf8').matchAll(LINK)) {
    let target = m[1];
    if (/^(https?:|mailto:|#|data:)/i.test(target)) continue;
    const a = target.indexOf('#');
    if (a >= 0) target = target.slice(0, a);
    if (!target) continue;
    let decoded = target;
    try { decoded = decodeURIComponent(target); } catch {}
    checked++;
    if (!fs.existsSync(path.resolve(path.dirname(f), decoded))) {
      broken.push(`${path.relative(ROOT, f).replace(/\\/g, '/')}  ->  ${m[1]}`);
    }
  }
}
console.log(`检查 ${files.length} 个文档，${checked} 条相对链接`);
if (broken.length) { console.log(`\n断链 ${broken.length} 条：`); for (const b of broken) console.log('  ' + b); process.exit(1); }
console.log('断链 0 条 ✅');
