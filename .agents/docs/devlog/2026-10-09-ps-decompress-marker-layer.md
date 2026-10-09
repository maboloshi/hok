# PS 解压兼容层：自递归别名 P0，与它掩盖的两个静默错

> 2026-10-09 ｜ 提交 `750a51e` ｜ worktree `.worktrees/x3-ps-decompress-fix` ｜ 分支 `fix/ps-decompress-layer`

## 复现推翻了审计前提

X3 原本要评估三条「偏离」（`-Switches` / `-ExtractDir` / `-Removal`），端到端复现后发现它们**都不可达**。
真正的缺陷是 `asset_scripts/decompress.ps1:37` 那行「别名」：

```powershell
function Expand-7ZipArchive { Expand-7zipArchive @args }   # 想当大小写变体的别名
```

**PowerShell 函数名大小写不敏感**，所以它不是别名，而是把 `:15` 的真实实现**覆盖**成自递归。自 `c3250d6` 起：
**独立 hok**（无 Scoop lib）下 7z 家族脚本阶段一律 call-depth overflow（约 1332 个清单的脚本阶段）；
**装了 Scoop 的机器**上，脚本随后 dot-source `apps\scoop\current\lib\*.ps1` 把真实现盖回来 ——
走 Scoop 语义、**hok 的整个 `[[HOK_EXTRACT]]` marker 路径不执行**。本机正是后者，所以这个 P0 长期不可见。

## 修了什么

- 删掉那行，并在文件头写明为什么**不能**再写大小写变体包装。
- marker 从 `format|source|dest[|removal]` 改为五段 `format|source|dest|extract_dir|removal`：
  由 Rust 侧应用 `extract_dir`（只取子树再上提，对齐官方 `lib/decompress.ps1:96-99,187-214`），
  并**仅在解压成功后**删源 —— `-Removal` 原先在 PS 侧先删源，Rust 读到 marker 时 `source.exists()` 已为假而**静默跳过**。
- 读取端补 `[[HOK_EXTRACT]]` 前缀与 PS 5.1 BOM 处理。此前 `format` 是带前缀的整串 ⇒ `innosetup` 分支**从未命中**，
  且 `Out-File -Encoding ASCII` 把非 ASCII 路径写成 `?`，Rust 侧因源不存在而跳过。
- `-Removal` 改成 `[Switch]`：原先它不是开关，而所有清单都写裸 `-Removal`。
- 四个函数补官方默认值 `$DestinationPath = (Split-Path $Path)`（用户追加批准）：此前只给 `-Path` 的调用把 dest 留空，
  空 dest 经 `dest.join(name)` 变成**相对路径**，于是解到**用户运行 hok 时的 CWD**。语料实测 156 条调用行 / 88 个清单属此列。
  **变异测试意外实证了这条**：临时去掉默认值后，成员真的落到了进程 CWD，泄漏出 `crates/libscoop/app/`，
  连带弄红了无关的 `package::info::tests::lookup_prefers_the_global_install`（它的查询串就是字面量 `"app"`，
  而该目录恰好存在了）—— 不是基座变红，是自己泄漏的产物。
- **有意不跟**官方 `:93` 的 `$DestinationPath.TrimEnd('\')`：官方是为了交给 `7z -o`，hok 的值只当路径用；
  `C:\pkg.7z` 经 `Split-Path` 得 `C:\`，TrimEnd 会把它变成盘符相对路径 `C:`。

## 证据

隔离根（`SCOOP`/`SCOOP_CACHE`/`USERPROFILE` 全部重定向，真机 `$env:SCOOP` 未写）：
`hok install todesk --offline -y -i` 由 exit 1 → **exit 0**，`ToDesk.exe` 在位、`app64.7z` 已删；
`-ExtractDir` 样本的解压树与官方 Scoop 对同一归档的产出**完全一致**。
回归测试用 dot-source + `(Get-Command …).Definition` 真绑定三个拼写：把冲突行放回去即 exit 1（不是子串断言）。
门禁：`clippy -p libscoop` 0、`test -p libscoop` 0（863 passed / 0 failed）、`test -p hok` 0、改动文件 rustfmt 0。
**残留**：`-Switches`（74 行）与 `-Overwrite`（24 行）仍被忽略 —— 登记为 X5，不属本次批准范围。
