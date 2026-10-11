# `-d` 写盘护栏

> 2026-09-13 ｜ 会话 `dflag-write-guard`，已闭环，2 提交，分支 `fix/dflag-write-guard` → 已线性 ff 进 `refactor/edition-2024`，tip `be0092df`

**规则（一条，统一适用）**：`-d/--dir` 缺省仍为 `.`；但**本次调用会写盘**时必须显式指定目标，否则 exit 1 —— **由写盘与否决定，不由命令身份决定**。
- `formatjson` 总是写 → 恒要求 `-d`；`checkhashes` 写盘 = `-u || -f`；`checkver` = `-u || -f`（官方 `if ($ForceUpdate) { $Update = $true }`，`checkhashes.ps1:52` / `checkver.ps1:404`）。
- `checkurls`/`missing-checkver` 从不写 → 保持默认 `.`（**有意放宽**：官方这几个也强制 `-Dir`）。
- `ci-auto-pr` 未改，**规则同样适用**（其 `-d` 现默认 `bucket`，`src/cmd/auto_pr.rs:21-23`）。
- `formatjson` 写盘时**逐文件**输出 `Formatting <path>...`（官方静默 ⇒ 有意偏离/hok 增值）。
**实现入口**：`src/cmd/shared_args.rs` 的 **`write_dir`/`scan_dir`**（新增写盘命令请复用，**勿再各写一份判据**）；`dir` 字段一律 `Option<PathBuf>`（写盘 `ok_or`、只读 `unwrap_or(".")`）。
**破坏性变更**：裸 `hok formatjson`、`hok checkhashes -u|-f`、`hok checkver -u|-f` 由"扫 cwd"变 exit 1。
**实测（护栏前的危险行为）**：隔离环境裸跑 `hok formatjson` 会重写**诱饵树里所有内容有变化的合法 JSON** —— `tsconfig.json`、`package-lock.json`、`target\debug\out.json`，连隔离出的 `~/.config/hok/config.json` 也被改写。根因：`manifest_walker.rs:44-45` 递归收全部 `*.json`（仅排除 `package.json`），`formatjson.rs:57-70` 走通用 `serde_json::Value`、**不判断"像不像 Scoop 清单"**。官方四脚本 `-Dir` 均 `Mandatory=$true`（非交互 exit 1），`checkver.ps1:85` throw。
**既有问题（未修，建议另开会话）**：`hok checkver` 在**没有任何清单声明 checkver（含空目录）**时**无限挂起** —— 已在 base `2a18d5b` 复现，与本护栏无关（D4 会话也独立发现同一现象）。
