# `.parts` 残留分片清理：`remove_incomplete` 补上分片下载那半

> 2026-09-25 ｜ 1 提交 `7b92635`，worktree `.worktrees/s-cleanup-parts`，分支 `fix/cleanup-parts`

**起因**：接上一条「`cleanup -k` 缓存面三修」的收尾盘点——`.parts` 分片残留目录 `cleanup -k` / `remove_incomplete` 都不覆盖，用户拍板"顺手补"。

**根因**：分片下载把分块写进 `<缓存名>.parts`，**只有重组成功那一步**才删这个目录（`download.rs:135-154` 的 `reassemble_fragments` 逐个删 `part.N` 后 `remove_dir`）。被中断（Ctrl-C / 崩溃 / 下载失败）就只剩分块、**连缓存文件都还没生成**，于是三条常规路径全都够不着它：① `remove_outdated` 走 `list`，而 `list` 只收**文件**、且只针对**已安装** app 的非当前版本；② 官方那行 `Remove-Item "$cachedir\$app#*"` **不带 `-Recurse`**，对非空目录直接报错跳过——上游同样清不掉；③ `remove_incomplete` 是唯一专门找"半成品"的入口，却只按扩展名匹配 `*.download` **文件**。

**改动**（`crates/libscoop/src/cache.rs` + `src/cmd/cleanup.rs` + 两份 locale）：
- `remove_incomplete` 现在同时扫 `<缓存名>.parts` **目录**（用同一个 `REGEX_CACHE_FILE` 判"是不是缓存名形状"，所以只是碰巧以 `.parts` 结尾的用户目录不动）；返回 `IncompleteSweep { entries, bytes }`，字节数**递归求和**（目录自身 `metadata().len()` 是 0）。
- CLI 另起一行报它（新键 `cmd.cleanup_incomplete_removed`）：半成品不是"过期条目"，且"只删了分片"的运行不能让用户看着像什么都没发生。已有那行 `cmd.cleanup_cache_removed` 语义不变。
- `remove_entry` 改为返回"本次是否真的删掉了"，让汇总只累加**真正释放**的字节（并发下载刚好自己清完 → NotFound → 不计入）。
- 并发安全说明：正在写分片的下载进程持有文件锁，递归删除会失败并走既有的 `cmd.failed_remove_cache` 报错通道，不会把别人正在下的分片悄悄挖掉。

**验证**：worktree `cargo test -p libscoop` **787 passed**（新增 `remove_incomplete_sweeps_part_dirs`：11+5 字节分块 + 3 字节 `.download` → `IncompleteSweep{entries:2, bytes:18}`，`notes.parts/` 保留）、`cargo test -p hok` 全绿、`cargo clippy --workspace --all-targets --exclude innospect --no-deps -- -D warnings` 干净、改动文件 `rustfmt --edition 2024 --check` 干净。隔离 `SCOOP` 根端到端：`authpass#1.10#ccccccc.exe.parts`（**当前版本**的分片——`-k` 按设计保留当前版本缓存，所以只有这个入口够得着）→ `残留下载分片已清理：共 1 项，3 KB`；过期缓存 + `.download` 归第一行（`共 2 个文件，513 B`）；当前版本缓存文件、其它 app 缓存、用户目录 `notes.parts/` 全保留。落地 `refactor/edition-2024`（`7b92635`，脚本漂移校验：1 提交 / 主题 / 4 文件集守恒 + 树逐字节相同，无 merge commit）。

**坑/教训**：① **"只有成功路径清理临时目录"= 中断即永久泄漏**：分片目录由写入方自己删，进程一死就再没人认领，且它**不属于任何已完成缓存条目**（没有缓存文件做锚点）——这类"孤儿临时态"必须有一个按目录形状扫描的兜底入口，而不是挂在条目删除流程上。② 官方同样修不了（`Remove-Item` 不带 `-Recurse` 对非空目录无效），所以这是**有意的 hok 扩展**，已登记。③ 报字节数就得**递归求和**：目录的 `metadata().len()` 在 Windows 上是 0，"释放 0 B"的汇总行会骗人。④ `remove_entry` 只返回"删除是否成功"还不够——`NotFound`（并发下自己清完了）也算"不再存在"，但要计入"我释放的字节"就必须区分，故返回"**本次**是否删掉"。
