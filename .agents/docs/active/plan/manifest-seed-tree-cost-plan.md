# 清单种子树 diff 成本：交接与实施方案

Status: active
Updated: 2026-10-06

> **给新对话（唯一入口）**：先读 §1 结论与 §7 交接提示，再按 §4 实现、§5 验收。
> 相关工作区：worktree `.worktrees/mhi-history-index`、分支 `feat/manifest-history-index`；本主题是**当前 tip 之上**的一条新提交。
> 本主题只动 `crates/libscoop/src/internal/git.rs` 的树 diff 原语与 `package/manifest_history.rs` 的分块消费方式，
> **应当是那 7 条提交之上的一条新提交**——不要改动已落地提交的语义，也不要混进它们的历史。
> 索引本体（表/种子/读取）的方案见 [`manifest-history-index-plan.md`](manifest-history-index-plan.md)。

## 1. 一句话

种子的 93% 花在「每提交一次树 diff」上。2026-10-06 的拆解实验证明这笔钱由两部分构成，且**两部分都能省掉**：

1. 现状**每提交把同一棵 `bucket/` 树读两遍**（第 N 提交当新侧、第 N+1 提交当旧侧），而 libgit2 只缓存 ≤4 KB 的 tree ⇒ 永不命中；
2. libgit2 的 diff 引擎在加载之外**还要逐条目扫两棵树**（~4,810 项/提交）来找变化。

「父树记忆化（每提交只读一次子树）＋ 自研 merge-join」原型把单线程耗时打到
**extras 181.3 s → 77.4 s（2.3×）、main 107.1 s → 44.8 s（2.4×）**。

## 2. 实测拆解（单线程、纯 git2 原型；2026-10-06 本机 Ryzen 7 6800H / 8 物理核）

| 阶段 | extras（92,615 提交） | main（81,452） | 每提交 |
|---|---|---|---|
| 父链游走（只读 commit 对象） | 1.6 s | 1.4 s | 17 µs |
| ＋两个根树 | 4.0 s | 3.4 s | 43 µs |
| ＋**一个** `bucket/` 子树（102 KB / 2,405 项；main 68 KB / 1,664 项） | 57.1 s | 32.2 s | 0.62 ms |
| ＋两个子树（现状 diff 内部的读法） | 100.3 s | 62.4 s | 1.15 ms |
| 现状 `diff_tree_to_tree` 全程（= 当前产品） | 181.3 s | 107.1 s | 1.96 ms |
| **原型：父树记忆化 ＋ merge-join** | **77.4 s** | **44.8 s** | 0.84 ms |

- 一棵子树加载 ≈0.57 ms，成本在**逐条目分配 `git_tree_entry`＋复制名字**（2,405 项 ≈5.5 µs/项），**不是** delta 链解析。
- 记忆化父树即可把「两个子树」砍成「一个」：100.3 → 57.1 s（实测 memo 命中 86,996/92,615，未命中的正是没有 `bucket/` 目录的那段历史，见 §4）。
- 自研 merge-join 替代 libgit2 的 diff 引擎：再省 ~20 s（extras）/ ~12 s（main）。
- 现状并行基线（本次复测）：extras **32.3 s**（16 逻辑线程）、4 线程 70.2 s。本主题落地后预计 extras ≈13 s；**4 核机器 70.2 → ~30 s**（这是本主题的主要价值：给低核机器留余量）。

## 3. 已排除（不要重复踩）

- **tree 与父相同就跳过**：实测 **0%**（extras 24/92,615、main 15/81,453）⇒ 无收益。
- **完全不碰 `bucket/` 的提交跳过**：6.2% / 9.4% ⇒ 最多省 6–9%，不值。
- **分块大小**：500 / 2000 / 100 三档并行都是 ~33 s ⇒ 无差，保持 500。
- **libgit2 对象缓存**：把 tree 上限提到 256 KB ⇒ **51.6 s vs 基线 32.3 s（更慢）**；另一轮「16 MB ＋ `CACHE_MAX_SIZE`=1 GB」也是 229.2 s vs 119.7 s。libgit2 1.8.1 没有 diff tree cache，pack 侧 delta 基缓存是编译期常量（`pack.h`：16 MB / 1 MB），运行期没有旋钮。**这条永久排除**，也不值得再引入 `unsafe` FFI。
- **按物理核降线程**：16→8 = 41.1 s、16→4 = 70.2 s ⇒ 超线程在这里真有用，别按物理核限制。
- **懒索引 / 按 app 回放**：libgit2 没有 pickaxe，逐提交 `Tree::get_path` 同样要读那棵 100 KB 的 `bucket/` 树 ⇒ 单次查询 ~90 s，比全量种子更慢。
- **换后端 gix**：整仓迁移级别的主题，不在本主题内。

## 4. 方案（待实现）

1. `internal/git.rs` 增加「带记忆化的树 diff」：按目录层递归比较两棵树的条目（`name_bytes` ＋ `id` ＋ 文件模式），产出既有的 `HistoryDelta`。
2. `package/manifest_history.rs` 的并行分块改为**每线程连续切片**（`par_chunks` 或等价），让「上一轮那把子树句柄」能被本轮复用；记忆化 key 用子树 OID，`None`（目录不存在）也要缓存。
3. 保留的语义（不得回退）：
   - 过滤仍是「`.json` 且非仓库根 `package.json`」；
   - **必须按目录递归**：extras 最早的 5,620 个提交没有 `bucket/` 目录（清单在别的布局里），只 diff `bucket/` 子树会漏掉整段历史；
   - 覆盖新增/删除、blob 内容变更、模式位变更（100644↔100755）、file↔dir 类型变更、submodule（gitlink）条目。
4. ⚠️ **必须实现 git 的树条目排序规则**：目录名按 `name/` 参与比较（git `base_name_compare` 语义）。原型的 merge-join 用了朴素字节序，遇到含子目录的树会错位——它数出 **331 万** 个 delta，而真实只有 **97,929** 个（差 34×）。这是本主题的主要风险点，也是 §5.2 的第一条单测。
5. 记忆化的固有损耗：每个线程切片的第 1 个提交必然 miss（多读一次，可忽略）；布局切换（目录改名）会 miss。

## 5. 验收

1. **行为等价（硬门槛）**：同一真实桶上，新旧实现在 `(bucket, name, version, blob_oid, path, committed_at)` 上**逐行一致**（做法：把两张 `hok_manifest_history` 全表 dump 成有序文本再逐行 diff／哈希；2026-10-06 已用同一手法验证过「块大小无关」）。做不到就停下来报告，**不要**用「行为略有差异但更快」替代。
2. **语义单测**：含子目录的树能正确比对（**回归那个 34× 计数错**）、新增/删除、模式变更、file↔dir、`.json`/`package.json` 过滤、扁平与分类布局、根提交（空树一侧）。
3. **性能回归线**：隔离副本上 extras/main 的种子墙钟 ≤ 现状的 0.6×（即 ≥1.6× 提速），并把实测数字写回本文件与 devlog。
4. **门禁**：`cargo test -p libscoop --lib`、`cargo test -p hok`、`cargo clippy --workspace --all-targets -D warnings`、改动文件逐个 `rustfmt --edition 2024 --check` 全绿；**不新增任何 `#[allow(...)]`**。
5. 断点续跑 / 幂等 / 桶移除清理语义不变（现有测试必须继续全绿）。

## 6. 测量配方（复跑用；真实 `scoop.db` 一律只读）

```powershell
# 本机 scoop 根：`scoop config root_path` 的值（下称 $scoopRoot）
$scoopRoot = (Get-Content "$env:USERPROFILE\.config\hok\config.json" -Raw | ConvertFrom-Json).root_path
# 隔离副本：DB 复制 + 桶仓库复制 + hok.json
$dir = "$env:TEMP\mhi-bench\extras"
New-Item -ItemType Directory -Force "$dir\buckets" | Out-Null
Copy-Item "$scoopRoot\scoop.db" "$dir\scoop.db"
robocopy "$scoopRoot\buckets\extras" "$dir\buckets\extras" /E /NFL /NDL /NJH /NJS /NP
Set-Content "$dir\hok.json" ('{"root_path":"' + $dir.Replace('\','\\') + '","cache_path":"' + $dir.Replace('\','\\') + '\cache"}') -NoNewline -Encoding utf8NoBOM
# 每次冷启动都要重新复制 scoop.db，否则第二次是增量、几秒就结束
```

- **坑 1**：`buckets/<name>` 用 **junction 不行**——`bucket_added` 的 `entry.file_type().is_dir()` 会把 junction 当非目录跳过，报「没有桶」。必须真复制（extras 仅 97 MB、约 10 s）。
- **坑 2**：临时 bench 例若**完全不引用 `libscoop`**，链接会缺 `advapi32`/`shell32`（libssh2 的 Windows 后端）而失败；用一次真实调用（如 `libscoop::bucket::Bucket::from(path)`）把它带上链接行即可。
- **坑 3**：块大小/线程数的临时开关（`next_chunk` 里读环境变量、`rayon::ThreadPoolBuilder::new().num_threads(n).build_global()`）测完必须还原；本次已还原，`git status` 干净。
- 拆解探针用**纯 git2** 写成七个模式（`walk0/walk1/walk2/sub1/load1/load2/own`），每个模式只比上一步多做「每提交多读一棵树」，并打印 `memo-hits` 证明记忆化命中——这样任何一笔成本都能归因。
- 真实桶：`$scoopRoot\buckets\{extras,main}`（非浅克隆；extras 92,615 提交、main 81,452 —— 用 `git rev-list --first-parent --count HEAD` 复核）。

## 7. 交接提示

- 本主题请在**新对话**里开，沿用 worktree `.worktrees/mhi-history-index` 与分支 `feat/manifest-history-index`；新提交接在**当前 tip** 之上（先 `git log --oneline -1` 复核，交接时共 9 条：7 条索引本体 + 2 条本文件的交接提交）。
- 建议顺序：先写「带记忆化的树 diff」原语 ＋ §5.2 语义单测 → 再切 `manifest_history` 的消费方式 → 最后跑 §5.1 等价与 §5.3 性能线。
- 若等价做不到，或语义单测暴露无法在改动范围内解决的差异，**停下来报告**，不要降级验收。
- 背景：索引本体方案 [`manifest-history-index-plan.md`](manifest-history-index-plan.md)（含 §2 成本更正）；本次实验记录见 devlog [`../devlog/2026-10-06-manifest-history-index.md`](../devlog/2026-10-06-manifest-history-index.md)。
