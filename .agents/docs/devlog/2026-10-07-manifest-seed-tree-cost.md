# 种子树 diff 记忆化：等价逐字节通过，性能线在 16 线程机器上按字面未过

> 2026-10-07 ｜ 提交 `6a93ecc` ｜ worktree `.worktrees/mhi-history-index` ｜ 分支 `feat/manifest-history-index`

## 落地项

- `internal/git.rs`：`HistoryRepo` 增加按树 OID 的 `TreeCache`（容量 8）与 `TreeData`（名字单一缓冲、条目定长，两次分配不增长）；
  `manifest_deltas` 换成自研 `diff_trees` 递归比较（`base_name_compare` 按 git 语义、OID 相同的子树直接跳过），每次提交的 delta 末尾按路径**稳定排序**以恢复 libgit2 的发射顺序；libgit2 版 `manifest_deltas_of` 删除。
- `package/manifest_history.rs`：并行改为 `par_iter().with_min_len(16)`，一个任务是一段连续提交——记忆化才可能把「上一步旧侧」交给「下一步新侧」。
- 单测：与 `diff_tree_to_tree` 的**逐提交逐项**对照（覆盖子目录、`foo/` 与 `foo.json` 名冲突、增删、模式位、file↔dir、gitlink、根提交、`package.json`），外加 34× 计数回归、类型变更、目录展开/子模块、记忆化四个定点测试。

## 实测（§6 隔离副本；真实 `scoop.db` 全程只读）

- **§5.1 逐行等价：两个桶全表 dump 逐字节相同**——extras 92,616 提交/82,674 行、main 81,453/75,913（含基线表）。
- extras 种子墙钟（同机现测，括号为比值）：1 线程 208.9→**97.4 s**（0.47×）、4 线程 72.1→39.0（0.54×）、8 线程 43.1→26.0（0.60×）、16 线程 34.3→23.9（0.70×）；main 16 线程 21.0–23.0→13.7–15.2 s。
- 单线程 2.15×（§1 原型 2.34×，机器当时更慢），低核机器越线；16 线程只有 ~1.4–1.5×。

## 发现（都已写回方案 §8/§9）

1. **这笔钱在 ODB 解压，不在「逐条目分配＋复制名字」**：实测 `find_tree` 1.37 ms vs 裸 `odb.read` 1.52 ms（extras 当前 `bucket/` 子树，2,405 项）。据此自研裸解析该对象**无收益**，该优化已排除。
2. **16 线程下 §5.3 的 0.6× 近乎不可达**：现状在 16 线程已把 diff 引擎的扫描完全隐藏，剩下 45% 是每提交一次的子树 ODB 读取（关掉记忆化即 33 s，可见缓存生效）；即使 merge-join 归零，墙钟也只能到 ≈20 s（0.59×）。
3. 排除：加大块/任务粒度（2000/32 与 500/16 无差）、预分配（无差，保留）、升级 git2（1.8.1→1.9.7 该路径零改动，真修复 libgit2 PR #6863 未合并，且 0.20 起要显式开 ssh/https 特性）。

## 门禁

`cargo test -p libscoop -p hok` 847 通过（0 失败）；`cargo clippy --workspace --all-targets -- -D warnings` 干净；改动文件 `rustfmt --edition 2024 --check` 干净。

## 未落地 / 去向

- §5.3 的 0.6× 线在 16 线程机器上未达（0.70×），**未降级验收**：等价与语义无任何折扣，低核机器（≤8 线程）已过线；若这条线必须按字面满足，只剩「merge-join 归零」级别的不现实余量，需要重新评估该线本身。
- `pending-tasks.md` §3「上游 0.6.0 历史清单链」行仍未回填——该文件是汇总型热点，只在主树改，留待落地会话处理。
