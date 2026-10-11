# 清单缓存静默漂移修复：marker 基线 + 提示层

> 2026-09-12 ｜ 提交 `7b1effe` `8a34463`

**现象**：`hok info uv` 显示 `0.12.13（可更新到 0.12.12）`（官方 `scoop info` 只显示 `0.12.13`）。

**定位（证据）**：
1. 官方 `scoop info` 读清单**文件**（`scoop-info.ps1:62`）；hok 的 `info::lookup` 走 `query → query_synced → query_synced_cached`，即 **SQLite 缓存**（`use_sqlite_cache=true`）。
2. 复制 `D:\App\Scoop\scoop.db` 只读检查：`uv` 两桶都是 **0.12.12**（文件 0.12.13）；全库 21,372 行中 **82 行落后于文件**（`gimp 3.2.4→3.2.6`、`aws 2.36.42→2.36.43` …）。`(name,bucket)` 无重复版本行 ⇒ 不是历史堆积，是单行过期。
3. 过期永不自愈，三因叠加：
   - `bucket.rs` 的增量 diff 对比「**本次 pull 前**的 HEAD」→ pull 是 no-op 时 diff 为空，那些清单**再也不会**被 upsert；
   - `manifest_cache::refresh` 用 `let _ = populate/apply_incremental` **吞掉所有错误**；
   - 读路径零新鲜度校验（schema 与官方一致，无 mtime/size）。
4. 沙箱内复现不出：沙箱禁止 SQLite 建 `-wal/-shm` → `query_synced_cached` 静默回落文件路径。**danger-full-access 下当场复现**（这也是本轮定位的关键）。

**修复（方案 2，用户拍板；不做读时改走文件）**：
- 新增 hok 专属 side table `hok_manifest_sync(bucket, synced_commit)`（官方只读写 `app`，额外表兼容）；`bucket::update` 的 diff 基线改为**标记里上次成功同步的 commit**，pull 后把新 HEAD 记入 `changes.synced`；无标记（首次/失败后）⇒ 该桶 `Full` ⇒ 全量 `populate` ⇒ **自愈已有漂移**。
- `refresh()` 不再吞错：返回 `RefreshOutcome{Disabled,Incremental,Rebuilt}`，失败清空标记并返回 `Err`；`update.rs` 打 `ERROR 清单缓存刷新失败：{e}。请重试 'hok update --force'。` 后 `ExitCode(1)`（沿用 `shim`/`which` 的写法，避免顶部 handler 重复打印）。
- 顺带修：缓存未启用时不再假装打 `✓ 清单缓存已刷新。`；重建成功打 `✓ 清单缓存已重建。`。
- **提示层**：`query_synced_cached` 在服务缓存前用 `stale_buckets()`（标记缺失，或标记 != 当前 HEAD；无 git HEAD 的目录桶跳过）检测，命中则经 `session.output().warn` 每进程报一次：`WARN 清单缓存可能不是最新（4 个仓库未同步）；运行 'hok update --force' 重建。`（stderr，stdout 保持可 grep）。

**实测（本机）**：`hok update --force` → `✓ 清单缓存已重建。` → `hok info uv` = `0.12.13`、无提示；缓存漂移 **82 → 0**；标记表 4 桶齐全。

**坑**：
- SQLite 中 `commit` 是保留字，列名必须避开（改用 `synced_commit`），否则 `CREATE TABLE` 直接语法错误。
- 造「刷新失败」测试：`open()` 会 `CREATE TABLE IF NOT EXISTS app` 自愈，单纯 `DROP TABLE app` 造不出失败；要用**同名的畸形表**（`CREATE TABLE app (name TEXT)`）才能让 `populate` 的 INSERT 失败。
- SQLite 无法在沙箱（workspace-write）里打开：分析 `scoop.db` 必须**复制进工作区**（连同 `-wal` 判断），否则得到的是缺 WAL 的旧快照。

**附带发现（未处理）**：`scoop-app` 桶的 `bucket/` 里有非清单 JSON（`.markdownlint.json` = markdownlint 配置），被当成包缓存（版本空串）；10 条"疑似漂移"即此类，非真漂移。

## 后续（提交 `8a34463`）：按桶重建 + 非清单 JSON 跳过

**按桶重建**：原先任一无基线桶都会触发**全库** `populate`（21.7k 清单，实测 `hok update --force` 19.9s）。改为只重建被标记的桶（`DELETE FROM app WHERE bucket = ?` + 扫该桶），与增量 apply 同一事务；空缓存才走全库。本机实测：清掉 `scoop-private` 一个标记后 update **2.1s**（其中基本是 git pull），提示变成 `清单缓存已刷新（重建仓库：scoop-private）`。附带修一个既有泄漏：`bucket::remove` 从不碰缓存，过去靠"全库重建"兜底；按桶后必须显式 `DELETE FROM app WHERE bucket NOT IN (已加桶)`。

**非清单 JSON**：官方 `lib/database.ps1:176` 是 `if ($null -ne $manifest.version)` —— **没有顶层 `version` 的 JSON 不入库**；hok 之前只判"能否解析"，于是 `hok search appsettings` 会打出该配置文件（版本空）。现改为"版本为空 = 不可缓存"，走 `insert_manifest_row` 既有的 `None` 分支（会删同 `(name,bucket)` 的旧行），文件路径查询 `query_synced_files` 同步跳过。实测：本机 **11 行**空版本伪条目在下一次重建后清零，`search appsettings`/`markdownlint` 均无结果。

**未做但已评估（开放项，供后续决策）**：
1. **"命中即校验"**（`info` / `search -e`）—— 对返回的每条记录读一次文件比对版本，不一致就用文件值并提示。成本微秒级，效果是"`info` 永不给出错版本"，**不需要 mtime/size**（要判内容是否变，反正得读文件）。当前只做"漂移 WARN + 手动刷新"。
2. **mtime/size 行级记账**（当初方案 4）—— 唯一真收益场景是**本地目录桶（无 git）**：用"目录清单 + stat"算出增/删/改，把本地桶从"每次 update 全量重扫/完全无法检测漂移"变成可增量；以及标记丢失后的修复能只重读变化的文件。日常 git 桶无收益（commit 级 diff 更准更便宜），读时校验也不需要它。代价：side table + `populate`/`rescan_bucket`/`apply_incremental` 三处记账；且 mtime 对"保留时间戳的复制"会漏判，不能取代 diff。
3. **`hok update` 非 TTY 下桶 pull 失败不可见**（indicatif `ui.fail` 在管道里不渲染）——与官方"每桶失败可见"有偏差，待专项。

**性能参照（本机，21.7k 清单）**：全库重建 ≈ 19.9s（其中 scoop-app 17.6k 清单单独重扫 ≈ 25.9s，含 pull）；按桶重建单个小桶 ≈ 2.1s（含 pull）；常态增量 ≈ 1.9s；node 单线程 read+parse 全量 ≈ 13.6s（用于横向估算）。

**流程教训（同日）**：本条由「表格输出样式」话题中途的 `hok info uv` 版本异常引出，最终演成一个独立的缓存架构专项——用户指出**话题已远远偏离开场主题**，要求今后**强提醒另开对话**。已落为规则：`.agents/skills/hok-session-scope-guard`（本地，`.dsh` 被 gitignore）+ `AGENTS.md` Notes 指针 + Hindsight 记忆。后续开放项（命中即校验 / 本地桶 mtime 增量 / update 非 TTY 失败可见性）应另开对话处理。

---
