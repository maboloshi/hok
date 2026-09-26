#!/usr/bin/env node
// 生成 .agents/docs/devlog/README.md —— 日志索引（禁手改）
// 一条日志 = 一个文件 YYYY-MM-DD-<english-slug>.md
// 用法: node .agents/scripts/devlog-index.mjs [--check]
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DEVLOG = path.resolve(HERE, '..', 'docs', 'devlog');
const OUT = path.join(DEVLOG, 'README.md');

const HEADER = `# hok 开发日志

> **用途**：记录开发过程的关键决策、踩坑、性能数据与实验结论——面向维护者与后续开发会话。
> **定位**：与 \`CHANGELOG.md\`（用户可见变更）、\`.agents/skills/\`（agent 操作知识）互补；本文档回答"为什么这么做、踩过什么坑、数据是什么"。
> **CHANGELOG 分工**：\`CHANGELOG.md\` **仅在发布前统一更新**（从 devlog + git log 提炼，Keep a Changelog 风格按版本/类别整理）；开发过程中不动 CHANGELOG，所有过程记录写在这里。
> **维护**：一条日志 = 一个文件 \`YYYY-MM-DD-<english-slug>.md\`；首行 \`# <主题>\`，第 3 行 \`> <日期> ｜ 提交 … ｜ 分支 …\`。
> 本索引由 \`node .agents/scripts/devlog-index.mjs\` 生成，**禁手改**（规则见 [[docs/AGENTS.md]] R7）。
`;

const FILE_RE = /^(\d{4}-\d{2}-\d{2})-(.+)\.md$/;

/** @returns {{file:string, date:string, title:string, commits:string}[]} */
function collect() {
  const rows = [];
  for (const name of fs.readdirSync(DEVLOG)) {
    const m = FILE_RE.exec(name);
    if (!m) continue;
    const lines = fs.readFileSync(path.join(DEVLOG, name), 'utf8').split('\n');
    const titleLine = lines.find((l) => l.startsWith('# ')) ?? '';
    const metaLine = lines.find((l) => l.startsWith('> ')) ?? '';
    const commits = [...metaLine.matchAll(/`([0-9a-f]{7,40})`/g)].map((x) => x[1]);
    rows.push({
      file: name,
      date: m[1],
      title: titleLine.slice(2).trim() || m[2],
      commits: commits.join(' '),
    });
  }
  return rows.sort((a, b) => (a.file < b.file ? 1 : a.file > b.file ? -1 : 0));
}

const rows = collect();
const table = [
  '| 日期 | 主题 | 提交 | 文件 |',
  '|---|---|---|---|',
  ...rows.map(
    (r) =>
      `| ${r.date} | ${r.title} | ${r.commits ? '`' + r.commits.split(' ').join('` `') + '`' : '—'} | [\`${r.file}\`](${r.file}) |`,
  ),
].join('\n');

const content = `${HEADER}\n## 索引（${rows.length} 条，自动生成）\n\n${table}\n`;

if (process.argv.includes('--check')) {
  const cur = fs.existsSync(OUT) ? fs.readFileSync(OUT, 'utf8') : '';
  if (cur !== content) {
    console.error('devlog 索引已过期，请运行: node .agents/scripts/devlog-index.mjs');
    process.exit(1);
  }
  console.log(`devlog 索引一致（${rows.length} 条）`);
} else {
  fs.writeFileSync(OUT, content, 'utf8');
  console.log(`写入 ${path.relative(process.cwd(), OUT)}（${rows.length} 条）`);
}
