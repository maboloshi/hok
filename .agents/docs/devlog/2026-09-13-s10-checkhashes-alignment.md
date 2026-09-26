# S10 `checkhashes` 深度对齐

> 2026-09-13 ｜ 已闭环，4 提交，分支 `fix/s10-checkhashes` → 已线性 ff 进 `refactor/edition-2024`，tip `e97744a`

**结论（用户拍板）**：缓存 **A1**（**默认行为不动**——实测 hok 默认已是"每次重下 + 跑完删临时目录"＝官方默认；只把 `-k` 缓存键修为含版本 `<cache>/<version>/…`，且 `-k` ≡ 官方 `-UseCache`，`-f` 覆盖 `-k`）；空 hash **B1**（`""` 当 sha256 期望比较 → mismatch，`-u` 写回真实值；数组逐条独立、去掉 `break`；`TODO`/未知算法/对象形式改为**显式报解析失败**，**不放宽** `HashString` 解析层以免波及 install/check_hash）；url 选择 **C1**（顶层 `url` 优先，无顶层才 64/32/arm64，**新增** `Manifest::checkhashes_accessors()`，**不动** `all_urls()`）；小项全做（`-App` 大小写不敏感、`nightly` 大小写不敏感、无 URL 打印官方文案、明细补 `First bytes` + 完整 Expected/Actual、数量不匹配不再误记 failed）。
**实现**：`package/checkhashes.rs` 重写（`Entry{algorithm,expected,path,index}` + `formatted_expected()` + `collect_entries` + `update_manifest` + `magic_bytes_of` + 15 单测）；`manifest.rs` 新增 `checkhashes_accessors()`；`internal/string.rs` `matches_any_glob` 大小写不敏感（**与 S11 同源，合并保留超集**）；`src/cmd/checkhashes.rs` 状态分流。
**测试**：`cargo test --workspace` 仅 2 条既有 ACL 环境性失败；clippy 0 新增；改动文件 rustfmt 干净。
**Why:** 这 22 条里最有杀伤力的是"静默"：空 hash 让 `-u` 永远修不了、解析失败让坏 manifest 完全消失、`all_urls()` 合并让顶层+架构清单多查一遍假 mismatch、大小写敏感让 `-App` 在大写输入下静默扫 0 个。另外原审计那两条缓存结论是**静态读码误判**，实测（换服务端内容 + 数请求）才定案。
**How to apply:** ①url/hash 选择只走 `Manifest::checkhashes_accessors()`（**不要**再用 `all_urls()/all_hashes()`，那是 checkurls 的合并语义）；②空/非法 hash 一律**不静默**、不提前 `break`；③写回走 `internal::fs::write_json`（CRLF + 末尾换行），且只写有 mismatch 的清单；④`-k` 缓存**必须按版本分目录**，否则跨版本同 URL 会假 OK。
**有意偏离（登记）**：`handle_special_urls` 不做；`-d` 默认 `.` 与多 glob；末尾汇总行；缓存放 `%TEMP%\hok-checkhashes`（不污染用户真实缓存）；`-c/--cache` 扩展；下载失败继续；`-u` 只在有 mismatch 时写回（官方无条件重写**全部** manifest）；写回只动被改文件并保留原行尾。
**划出**：D4 `download_file` 缺 Scoop 请求头（UA/Referer/cookies/PRIVATE_HOSTS）→ 已另开会话 `fix/d4-download-headers`。
