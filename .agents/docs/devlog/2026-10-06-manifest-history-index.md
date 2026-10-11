# 清单历史索引落地：建表 / 种子与增量 / 读取三段，外加解析器 abort 前置修复

> 2026-10-06 ｜ 提交 `62261c5` `615a6bb` `61c3b7d` `27ce29e` `ab7c4c2` `9893616` ｜ worktree `.worktrees/mhi-history-index`（主线 `refactor/edition-2024`）｜ 分支 `feat/manifest-history-index`

## 落地项

- 六条提交即方案 §5 的 0–5：解析器前置修复 → 建表 DDL → 种子/增量（含 `hok update` 接入，失败只 warn）→ 读路径（`generate_user_manifest` 第 2 段 + 失效删行回落）+ `use_git_history` 从 `OFFICIAL_ONLY_KEYS` 迁为真实配置键（默认 `true`，只门控读取）→ 端到端链测试 → 方案落库。
- `internal/git.rs`：`HistoryRepo` 拆成 `walk_chain`（只读 commit 对象，给出提交与第一父的 tree OID）与 `manifest_deltas`（按两个 OID 做那次贵的树 diff）；OID 一律 40 位 hex，`package/` 不碰 git2 类型。
- `package/manifest_history.rs`：种子与增量共用「走链 + 500 提交一块并行 diff + 按链序单线程写库 + 按块边界推进游标」；版本只经窄探针 `VersionProbe`（仍走 `internal::json::parse_str`，JSON5 清单照样入索引）。
- 读路径 `manifest_history::find`：点查 + 读 blob；blob 被 `gc --prune` / 非 UTF-8 / 版本不符 ⇒ 删行并回落 autoupdate，不是报错；桶打不开则保行回落。

## 三个发现（都已进方案正文）

1. **scoop-app 种子 abort 的根因不是 json5，而是 hok 自己**：`License`/`Sourceforge` 的 `visit_map` 在 `next_entry`（已消费 key+value）之后又补一次 `next_value`；未知键在**末尾**时严格解析先报错、落到 json5 兜底，json5 对空队列 `unwrap`，而两个 profile 都是 `panic = "abort"` ⇒ 整个 `hok update` 进程死。真实触发只是 scoop-app 历史里 `bucket/pianobar_MCOfficer.json` 的 `"url:"` 笔误（官方 `validator.exe` 也判它 3 处 schema 错误）。`git blame` 指向主线既有提交 `f166d19`，故单列前置提交 `62261c5`。
2. **成本归因被实测推翻两次**：不是「解压 16 万个 blob 再解析 JSON」，而是逐提交的树 diff（main 109.6/119.7 s、extras 177.0/190.3 s，均 92–93%）；同一副本上原生 `git log --first-parent --raw` 只要 26.9 s(extras)，hok 慢 6.5×，且 `core.deltaBaseCacheLimit=0` 让 git 掉到 111.8 s ⇒ 本质是 pack 里相似 tree 的 delta 链解析。调 libgit2 对象缓存上限**更慢**（main 119.7→229.2 s），该路已排除。
3. **并行的实现坑**：`Repository` 必须每个 worker 线程只开一次（thread_local 缓存）。按「每块每线程 `open`」写时 main/versions 反而退化到 146.2/191.6 s（每次 open ≈28 ms × 数百块 × 线程数）；改为复用手柄后 19.2/18.1 s。

## 实测（隔离副本、真实桶；通过线 ≤60 s/桶）

| 桶 | 提交数 | 单线程 | 分块并行 |
|---|---|---|---|
| hp / scoop-private | 180 / 1,523 | 0.1 / 0.9 s | 0.1 / 0.3 s |
| scoop-app | 2,326 | 110.5 s | 39.6 s |
| main | 82,028 | 119.8 s | 19.2 s |
| versions | 106,541 | 96.9 s | 18.1 s |
| extras | 93,602 | 192.9 s | 30.2 s |

另做等价性对照：块大小 500 与「整链一块」两次运行的行数、内容哈希、基线逐字节一致（分块未丢行/未乱序）。真实 `scoop.db` 全程只读——测量一律用副本，前后字节数与 mtime 未变。

## 门禁

`cargo test -p libscoop --lib` 842 通过；`cargo test -p hok` 全绿；`cargo clippy --workspace --all-targets -D warnings` 干净；改动文件逐个 `rustfmt --edition 2024 --check` 干净。

## 后续（同会话追加实验）

树 diff 成本另做了一轮拆解实验（纯 git2 原型）：现状**每提交把同一棵 `bucket/` 树读两遍**，且 libgit2 的 diff 引擎还要逐条目扫两棵树，两笔各占约一半；「父树记忆化 ＋ 自研 merge-join」原型把 extras 181.3→**77.4 s**、main 107.1→**44.8 s**（2.3–2.4×）。同时排除：跳过未变更 tree（实测 0%）、子树 OID 短路（6–9%）、块大小（无差）、libgit2 对象缓存（两次实测都更慢）、按物理核降线程（16→4 掉出 60 s 线）。该主题**另开对话**处理，交接见 [`../reports/manifest-seed-tree-cost-plan.md`](../reports/manifest-seed-tree-cost-plan.md)。

## 未落地 / 去向

- 种子树 diff 成本优化（上节）：**已交接到新对话**，方案与验收在 `active/plan/manifest-seed-tree-cost-plan.md`。
- `pending-tasks.md` §3「上游 0.6.0 历史清单链」行待回填 ✅ —— 该文件是汇总型热点，只允许在主树改，落地时处理。
- 第一父链看不到 merge 中被丢掉的分支版本（上游 pickaxe 能查到）：已记入方案 §2 为已知取舍，触发信号是真实用户报告。
- 本方案文件落在 `active/plan/`；全部条目已落地，按 R4.4 可在落地后归档。
