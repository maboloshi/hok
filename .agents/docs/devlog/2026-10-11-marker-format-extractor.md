# marker 的 format 决定 extractor：改名下载不再静默失败

> 2026-10-11 ｜ 提交 `9f59e58` ｜ worktree `.worktrees/x6-marker-format` ｜ 分支 `fix/marker-format-extractor`

计划项 X6（X5 期间实测发现的相邻缺陷）。症状：`Expand-*Archive` 写下的 `[[HOK_EXTRACT]]` 标记里
`format` 字段**只被用来判 `innosetup`**，真正派发仍靠文件名后缀重嗅探（`operations/extract.rs` → `internal/archive.rs::detect_format`）。
于是官方改名的下载（`#/dl.7z_`、`#/dl.zip_`）在 marker 路径上以 `unknown archive format` **静默失败**——
语料受影响清单含 `cdex`、`hex-editor-neo`、`umbrello`、`dotnet-generic-unpacker`。

## 改法：marker 路径与原生路径分流

- **marker 路径**：`extract_markers` 不再调 `extract()`，改调 `Archive::extract_marker(format, …)` →
  `internal::archive::extract_by_marker_format`。映射即"脚本调用了哪个官方函数"：
  `innosetup` → inno 路由、`msi` → lessmsi、`zip`/`tar`/… → 对应内置解码器；
  **`7z` → 7z 家族**：名字认得（`detect_format` 有值）仍走原内置解码器（快路径），
  认不出（`.7z_`/`.zip_`/无后缀）→ `extract_pseudo_7z` 按**内容**嗅探（纯 7z / 7z-SFX / zip / NSIS / Inno），
  最终兜底 `7z.exe`。**未知 format → `extract_7z_cli`（官方 E7A 语义）**，绝不静默拷文件或 skip；
  7z.exe 缺失则响亮报错。⇒ 名字只用于在家族内挑更快的解码器，**不再决定是否解压**。
- **非 marker 路径零变更**：`extract()` 只是把原 `match` 抽成 `extract_by_format`，仍先 `detect_format`、
  未知名仍报 `unknown archive format`；`extract_archives` / `copy_downloaded_files` /
  `extract_script_cli`（X5 的 switches/overwrite 路径）未动。
- marker 七段协议布局**未改**（与并行进行的 X5b 无冲突）。

## 为什么保留 `7z` 家族的名字快路径

完全不看文件名，只能把无 switches 的 `7z` marker 一律交给 `extract_pseudo_7z` 或 7z.exe：
代价是 `.tar.gz`（无 `$IsTar` 二趟）会留内层 tar，`.zip`/`.rar`/`.msi` 会离开各自内置解码器并可能变慢。
当前实现已覆盖全部改名/无后缀场景且零回归，故不做更激进的改法（父会话判定）。

## 官方对照（本机 `../Scoop`）

- **自动解压按下载名选函数**：`lib/decompress.ps1:22-60`（`:26` `.zip`、`:34` `.msi`、`:38` `.exe`+innosetup、
  `:44 Test-7zipRequirement`，都不匹配则 `$extractFn = $null` **不自动解**）——正是 hok 非 marker 路径的对应物，
  故按名嗅探保持。
- **`#/dl.7z_` 为何走函数**：缓存名不匹配上述任一条 ⇒ 官方不自动解，由包脚本显式调 `Expand-7zipArchive`，
  而该函数**不看后缀**（`:94` 直接 `x <path> -o…`）⇒ marker 路径必须按 `format` 解。
- `Expand-MsiArchive:193`（lessmsi）、`Expand-InnoArchive:245-250`（innounp + `-c` from ExtractDir）同上。

## 证据与门禁

- 改名 + marker 真解压：`empty-entries.7z` 拷成 `dl.7z_`，marker `7z` → `plain.txt == "hello"` 且空条目/空目录齐全；
  `dl.zip_` 与"声明 zip 的改名文件"同样通过；未知 format 落 7z.exe。
- **地面真值交叉验证**：真 `7z.exe 26.04` 对同一 `dl.7z_` 的 `x` 输出与断言逐项一致。
- 非 marker 不变由既有用例钉住（`test_unknown_format` 仍 pin `detect_format("dl.7z_") == None`）。
- 门禁：`clippy -p libscoop --all-targets` 0、`test -p libscoop` 0（lib 884 passed / 0 failed）、两文件 `rustfmt --check` 0。
- **未验证**：完整 `hok install` 端到端（沙箱无出站 HTTPS）；未知-format 用例仅在"7z.exe 在位"下跑过。
- `is_native_format` 的名字表与 `extract_by_format` 的 match 是同源两份，用 `test_native_format_predicate_matches_dispatch` 钉同步。
