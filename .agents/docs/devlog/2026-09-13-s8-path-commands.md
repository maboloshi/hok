# S8 路径解析类

> 2026-09-13 ｜ which / prefix / cat / home，已闭环，提交 `c5b21d2`/`41ddaeb`/`4b25087`，分支 `fix/s8-path-commands` → 已线性 ff 进 `refactor/edition-2024`，tip `4b250876`

**结论（用户拍板）**：which=A1 完整对齐（真实目标 + PATH 程序 + 全局 shim，只输出一个）；cat/home=B1 单入口；prefix=D1；home 无 homepage=E1；**多 bucket 交互保留（暂缓/待定）**；cat 美化与 bat 回退=F2 有意偏离；**info 必须复用同一入口且零回归**。
**实现**：
- `crates/libscoop/src/package/manifest_source.rs`：`resolve_manifest` → **公开 `get_manifest` + `GetManifest`**，逐分支复刻 `lib/manifest.ps1:34-112`：①URL/UNC（UNC 按本地文件读，官方 `WebClient` 打不开）②已安装（`install.json` 的 `bucket` → bucket 清单 + `<bucket>\deprecated` 回退；无 bucket → `url` 远程/本地 → installed manifest；**一旦已安装绝不回落 bucket 分支**）③未安装（`parse_app` 拆 `bucket/name@version`、显式 bucket 只读该 bucket、否则逐 bucket 扫描、本地文件 `Test-Path`）。`GetManifest.candidates` 是 hok 扩展（多 bucket 候选，**按 bucket 名排序**，因为 `bucket_added` 并发无序），供 CLI 保留交互选择；`bucket` 形参是 hok 扩展。
- `crates/libscoop/src/package/shim.rs`：新增 `command_path`/`classify_command_path`/`shim_target`（`.shim` 首行 `path = `、其余读 `^(?:@rem|#)`、再兜底最后一段引号串；**目标不存在即 None** = 官方 `Convert-Path -ErrorAction SilentlyContinue`）+ 9 条单测。
- `crates/libscoop/src/shim.rs`：`resolve_in_path` 改返回 `Option<PathBuf>`，顺序 **`.ps1` → `%PATHEXT%`（缺省 `exe;cmd;bat;ps1`）→ 裸名**（三档均实测于 PS 5.1 + pwsh 7）；`shim add` 与 S7 的 `shim_info`(is_hidden) 共用它。
- `src/cmd/mod.rs`：`resolve_manifest_query`（唯一 CLI 侧解析助手，含多候选交互）；`cat`/`home` 只调它；`home` 在 `open_url` **之前**判空 homepage。
- `Session::app_current_dir_in(name, global)`：`prefix` 本地→全局两次 `currentdir` 探针（勿再用 `app_current_dir` 代两个作用域）。
**测试**：隔离 `SCOOP`/`SCOOP_GLOBAL`/`USERPROFILE`/`PATH` + 自建 apps/shims/buckets，22 条 which/prefix/cat/home 用例 + 3 条 download 解析用例；libscoop 单测 11+9 条；`info` 回归 12 用例（7zip/git/nodejs/uv/neovim 未安装/`bucket/name`/本地清单/not-found/`-v`）**对 base 二进制逐字节一致**（真实 root 只做元数据副本，**勿直连 `$env:SCOOP`**——hok 打开 `<root>\scoop.db` 会建表并切 WAL）。
**已知限制/有意偏离**：多 bucket 命中保留交互（**非 tty exit 1，重名包时脚本里不可用**）；不套 `friendly_path`；不建模 PS alias/函数、不查当前目录；**全局 `scoop-*.ps1` 别名反向偏离**（实测官方 `Get-CommandPath scoop-ghello` 返回 `$null`，hok 返回脚本路径）；cat 保持 2 空格/LF 与 bat 回退；UNC 按本地文件读。
**范围外副作用（勿忘）**：`download`/`install` 的 URL/本地/`@version` 解析随单入口一并改用 `Get-Manifest`（官方 `install.ps1:9`、`scoop-download.ps1:62` 亦然），即会读安装记录——**不要回退**，回退等于给 download 单开第二条解析路径。
**How to apply:** 清单来源解析只改 `package::manifest_source::get_manifest`（**不要再引入第二条 resolve 路径**；install/download 也用它）；`which` 的命令解析只改 `libscoop::shim::resolve_in_path` + `package::shim::command_path`；多候选的 CLI 交互只在 `cmd::resolve_manifest_query`；`prefix` 的作用域探针用 `Session::app_current_dir_in`。
