# 技能内容级审查：断言过期 / 重复 / description / 同名归档件

> 2026-09-26 ｜ 3 提交 `2806108` `1a9249a` `8c97220`，worktree 主树（仓库根），分支 `refactor/edition-2024`

**起因**：`.agents/skills/` 上一轮只做过**形态**扫描（日期章节 / 绝对路径 / 体积），断言是否仍为真没查过——而反复出错的全是这一类：已改名的函数、已删除的模块、已退役的依赖。

**方法**：16 个技能逐条核对它引用的代码 / API / 常量 / 文档，证据到 file:line。5 个只读子代理分片核对 + 父会话抽验（`Session` 到底是不是 `Sync`、`nsis` 目录语义归谁、`jsonpath` 的两处应用点）。

**发现（`2806108`）**：重灾区 6 个——
`hok-code-organization`（`operations/` 实为 extract/link/**psengine**/script；`Session` 自 RwLock/AtomicBool/OnceLock 重构后已是 `Sync + Send`，"不是 Sync 不能进并行闭包"的理由作废；分块与 `spawn_range_server` 已拆到 `download_fragmented.rs`；`verify_downloads` 在 `download_verify`）；
`hok-no-junction-implementation`（`select_current_version` 已不存在 → `install_state::resolve_current_version`；`no_junctions` 别名现被显式拒绝；`query_synced` 并不读已装 metadata）；
`hok-nsis-extraction`（目录语义已归上游 `nsis` 0.4 的 `dest_path()` + `to_install_path()`，本地三 pass 已删）；
`hok-jsonpath-hyphen-compat`（`extract_hash_from_page` 已不存在 → `json_path_first`）；
`hok-time-instead-of-jiff`（`rust-i18n` 整体移除、冷却调用点已删）；
`hok-shim-dual-variant`（`HOK_SHIM_GUI_BYTES`、21 个单测、官方锚点在 `lib/install.ps1` 而非 `core.ps1`）。
零漂移：git-pull / write-json / rust-gate / report-archive / local-scoop-source-path。另有 8 组行号漂移校正、13 条 description 改成"谁在什么场景加载"、纯记录日期与提交号清掉。

**去重（`1a9249a`）**：9 个知识点各写在 2+ 技能里（热点文件清单与"只在主树改"、detached HEAD 收口、PowerShell 提交信息、once 闩锁假 flaky、`query.rs` filter_map、去重门槛 + P2-6、归档/判据复述）。各收敛到唯一拥有者，别处只留指针。

**归档件（`8c97220`）**：`archive/skills/` 里 3 份与现役同名的旧副本，**入库当天就已落后**（38 vs 85、51 vs 72、32 vs 428 行），细节级比对确认无独有内容（`hok-audit-fix-history` 的三轮正文与 `reports/audit-fix-history-2026-08.md` 逐字相同；`scoop-gap-audit` 的 P0–P3 全在 `active/audit/scoop-gap-status.md`；`hok-code-organization` 的 `internal/` 名单停在 14 文件）→ 删除，git 历史保留。

**教训（最贵的一条）**：**批量字面替换不要用 JS `String.replace`**——替换串里的 `` $` `` / `$&` / `$'` / `$1` 会被当成"匹配上下文"展开。本次写入 `（`$` / 反引号）` 时，`` $` `` 把 `hok-audit-fix-history` 的正文整段注入它自己（86 行、2 份 frontmatter）。发现后写校验脚本切回，再 `git reset --soft` 重建两个提交并逐提交断言 `name=1 sep=2`，历史里不存在坏版本。正确写法：`split(old).join(new)`。

**未落地 / 去向**：代码侧同源错误（`internal/path.rs:64-67`、`package/query.rs:440-441` 仍写 "Session is not Sync"；`session.rs:370` 的 rustdoc 死链 `Session::version_dir`）→ 另开 Rust 主题；合并 `main` / push / 版本号 → 另开。技能里保持原样的 UNVERIFIABLE：NSIS 字节级实测、库选型下载量、性能断言。
