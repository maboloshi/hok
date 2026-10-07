#!/usr/bin/env node
// 生成 .agents/docs/devlog/README.md —— 日志索引（禁手改）
// 一条日志 = 一个文件 YYYY-MM-DD-<english-slug>.md
// 用法: node .agents/scripts/devlog-index.mjs [--check]
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..', '..');
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

/**
 * 每条日志「落地时间」（秒）：**新增该文件的那个提交**的 committer date。
 * 同一天多条时用它做 tiebreak——只按文件名排会退化成 slug 字母序，读起来像插错位置。
 * 取新增（--diff-filter=A）而非最后一次触碰：事后改个错字不该让老条目跳到当日最前。
 * stdout 走临时文件而非管道：DSH 沙箱下 Node 的管道 stdio 会 EPERM（见 .agents/AGENTS.md 已知环境坑）。
 * git 不可用（非仓库 / 无 git / 沙箱挡死）时返回空表，排序退化为 slug 序并告警。
 * @returns {Map<string, number>}
 */
function landingTimes() {
  const map = new Map();
  const tmp = path.join(os.tmpdir(), `devlog-index-${process.pid}.txt`);
  let fd;
  try {
    fd = fs.openSync(tmp, 'w');
    const r = spawnSync(
      'git',
      ['log', '--diff-filter=A', '--pretty=format:\u0001%ct', '--name-only', '--', path.relative(ROOT, DEVLOG).replace(/\\/g, '/')],
      { cwd: ROOT, stdio: ['ignore', fd, 'ignore'] },
    );
    fs.closeSync(fd);
    fd = undefined;
    if (r.error || r.status !== 0) throw r.error ?? new Error(`git log exit ${r.status}`);
    let t = 0;
    for (const raw of fs.readFileSync(tmp, 'utf8').split('\n')) {
      const line = raw.trim();
      if (line.startsWith('\u0001')) {
        t = Number(line.slice(1)) || 0;
      } else if (t && line) {
        // git 输出的是仓库相对路径；目录是扁平的，取 basename 与 readdir 对齐
        const name = path.posix.basename(line);
        if (!map.has(name)) map.set(name, t); // git log 新→旧，首次写入即该文件最近一次落地
      }
    }
  } catch (err) {
    console.error(`警告：取不到 git 落地时间（${err?.message ?? err}），同日条目退化为 slug 序`);
    map.clear();
  } finally {
    if (fd !== undefined) try { fs.closeSync(fd); } catch { /* 已关 */ }
    try { fs.unlinkSync(tmp); } catch { /* 没建起来 */ }
  }
  return map;
}

/** @returns {{file:string, date:string, title:string, commits:string}[]} */
function collect(times, now) {
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
  // 日期降序 → 同日按落地时间降序（未提交的条目视为最新）→ slug 降序兜底
  return rows.sort((a, b) => {
    if (a.date !== b.date) return a.date < b.date ? 1 : -1;
    const ta = times.get(a.file) ?? now;
    const tb = times.get(b.file) ?? now;
    if (ta !== tb) return tb - ta;
    return a.file < b.file ? 1 : a.file > b.file ? -1 : 0;
  });
}

const rows = collect(landingTimes(), Math.floor(Date.now() / 1000));
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
