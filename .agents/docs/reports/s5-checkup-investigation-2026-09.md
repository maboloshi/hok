# S5 `checkup` 语义决策 — 调研报告（2026-09-13）

> **范围**：`.agents/docs/active/plan/command-deviation-fix-plan-2026-09.md` §3「S5」；报告 §2 基线行、§0「最接近下一个 checkup」；`.agents/docs/active/audit/scoop-gap-analysis-2026-08.md` §4.4、§6。
> **对照源**：本地官方 `D:\App\github项目\Scoop\Scoop`：`libexec/scoop-checkup.ps1`（v0.5.3）+ `lib/diagnostic.ps1`；hok 侧当前工作树，主线 tip `14b9c29`。
> **状态**：**调研完成，待用户决策**（A 全面对齐 / B&C 并存 / D 不做）。

---

## 1. 结论先行

`checkup` 是**同名不同义**的极端案例：官方 = **环境诊断**，hok = **已装包结构体检**。二者都合理，且 hok 版本另有 1 个**不可达分支缺陷**。

**建议：方案 B/C（两者并存）** —— `checkup` 让给官方语义（环境诊断），hok 原有的已装包体检改名为新命令 `health`（保留其"指向 `hok reset <app>` 修复"的定位）。

理由（对齐 hok 的指导目标「比 Scoop 更好更快」）：
- 官方 `checkup` 是被用户/脚本广泛预期的命令名，语义错位成本长期存在；
- hok 的已装包体检有独立价值，直接删除是净损失；
- 改名只影响 hok 单一二进制内部命令名，无外部 schema/格式兼容负担。

---

## 2. 期望 vs 实际（逐检查项对照）

### 2.1 官方 `scoop checkup`（`libexec/scoop-checkup.ps1:11-56` + `lib/diagnostic.ps1`）

| # | 检查 | 判定逻辑 | 通过条件 |
|---|---|---|---|
| 1 | Defender 性能 ×2（local + global） | 仅当 **管理员** 且 `USERNAME != WDAGUtilityAccount` 才做（`scoop-checkup.ps1:11-16`） | `DisableRealtimeMonitoring=true` → 通过；否则 WinDefend 服务 Running 且排除路径含 scoop 根 → 通过；否则 warn ×2（`diagnostic.ps1:6-27`） |
| 2 | main bucket 已添加 | `Get-LocalBucket -notcontains 'main'` | 在列表内（`diagnostic.ps1:29-38`） |
| 3 | 长路径 | `LongPathsEnabled`（`HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem`）为 1，且 OS build ≥ 1607 | 已启用（`diagnostic.ps1:40-55`） |
| 4 | 开发者模式 | `AllowDevelopmentWithoutDevLicense`（`HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock`）为 1 | 已启用（`diagnostic.ps1:57-69`） |
| 5 | 7zip | 未安装且 `get_config USE_EXTERNAL_7ZIP` 为假 → warn | 已装 / 用外部 7zip（`scoop-checkup.ps1:22-25`） |
| 6 | innounp | `Test-HelperInstalled -Helper Innounp` | 已装（`scoop-checkup.ps1:27-30`） |
| 7 | dark | `Test-HelperInstalled -Helper Dark` | 已装（`scoop-checkup.ps1:32-35`） |
| 8 | 全局根卷 | `$globaldir` 所在卷 `DriveFormat -ne 'NTFS'` → error | NTFS（`scoop-checkup.ps1:37-41`） |
| 9 | 根卷 | `$scoopdir` 所在卷 `DriveFormat -ne 'NTFS'` → error | NTFS（`scoop-checkup.ps1:43-47`） |

**汇总与退出码**（`scoop-checkup.ps1:49-58`）：
- `issues>0` → `warn "Found N potential problem(s)."`；
- 否则 `defenderIssues>0` → `info "Found N performance problem(s)."` + `warn "Security is more important than performance, in most cases."`；
- 否则 `success "No problems identified!"`；
- **始终 `exit 0`**。

### 2.2 hok `checkup`（现状）

`src/cmd/checkup.rs:12-34` → `libscoop::package::checkup::check_installed`（`crates/libscoop/src/package/checkup.rs:47-114`）：
- 扫描 `<root>/apps/*`，逐包检查 `current` junction 存在（`no_junction` 模式下改为解析版本目录）+ `install.json` + `manifest.json`；跳过保留目录 `scoop`；
- 有 issue → 逐条 `name: message` + `N issue(s) found.` + `run 'hok reset <app>' to fix`；无 issue → `No issues found.`；
- 退出码 0。

**已确认缺陷（不可达分支）**：`src/cmd/checkup.rs:19-22` 的 `if !apps_dir.exists() → "No apps directory found."` 永远不可达——`check_installed` 在 `apps_dir` 不存在时返回空列表（`checkup.rs:51-53`），于是先命中 `issues.is_empty()` 分支。这正是技能记忆里记录的"checkup unreachable branch"微缺陷。

### 2.3 差异清单

| 维度 | 官方 | hok 现状 | 类别 |
|---|---|---|---|
| 语义 | 环境诊断（9 项） | 已装包结构体检 | 语义不同（基线） |
| 命令名冲突 | `checkup` | `checkup` | — |
| 与 `reset` 的关系 | 无 | 输出建议 `hok reset <app>` | hok 独有 |
| 缺陷 | — | `!apps_dir.exists()` 分支不可达 | bug |

---

## 3. 候选方案

| 方案 | 做法 | 代价 | 破坏性 |
|---|---|---|---|
| **A 全面对齐（替换）** | `checkup` = 官方环境诊断；**删除**已装包体检 | 丢失 hok 独有体检入口；`checkup.rs` 现有单测删除 | 中（功能净减少） |
| **B/C 两者并存** ⭐ | `checkup` = 官方环境诊断；新增 `health` = 已装包体检（原逻辑迁入并修掉不可达分支） | 新增 1 个 hok 独有命令 + 文案/文档/技能清单更新 | 中（`checkup` 输出语义变更，但正是对齐目的） |
| **D 不做** | 保持 hok 语义，登记为有意偏离 | 同名不同义长期存在；用户/脚本预期错位 | 无 |

> 计划里 B（健康检查改名 `health`、`checkup` 腾给官方）与 C（两者并存）在**影响面与结果上等价**，合并为一个方案。真正的三选一是 A / B&C / D。

---

## 4. 实现要点与代价（方案 B/C）

### 4.1 命令与模块改动
- `src/cmd/checkup.rs`：改为环境诊断入口；新增 `src/cmd/health.rs`（迁移原 `check_installed` 逻辑），`src/cmd/mod.rs` 加 `Health` variant + dispatch（`build.rs` 自动注册模块，仍需手动加 enum/match）。
- libscoop：保留 `package/checkup.rs`（体检，接口不变，供 `health` 用）；新增环境诊断模块（建议 `package/diagnostic.rs` 或 `internal/diagnostic.rs`），`package/mod.rs` 导出。
- `locales/en.yml` / `zh.yml`：`checkup` 新文案 + `health` 文案（中英结构对齐）；`tests/i18n_keys.rs` 校验键一致性。

### 4.2 环境检查所需基础设施（可用性已核实）

| 检查 | 现有能力 | 结论 |
|---|---|---|
| 管理员 | `Session::is_admin()` → `internal::os::is_admin()`（`session.rs:255`，shell32 `IsUserAnAdmin`） | ✅ 直接可用 |
| main bucket | `bucket::list(session)`（`crates/libscoop/src/bucket.rs:462`） | ✅ 直接可用 |
| 长路径 / 开发者模式 | `winreg = "0.56"` 已是依赖（`crates/libscoop/Cargo.toml:70`；`internal/env.rs` 已用） | ✅ 直接可用 |
| 7zip/innounp/dark | 逻辑已存在但为**私有**：`package/resolve.rs:38 helper_installed`（`HELPERS` 数组，基于 `install_state::load_install_state`） | ✅ 抽成公开 `helper_installed(session, name)` 即可复用 |
| `USE_EXTERNAL_7ZIP` | hok 不建模该键（`config.rs` 视为不支持，`unsupported_keys_rejected` 锁定） | N/A —— 恒按"未使用外部 7zip"处理，需在文档注明 |
| NTFS 卷 | `windows-sys = "0.61"`（features 含 `Win32_Storage_FileSystem`）；需补 `GetVolumeInformationW` FFI | ⚠️ 中等，几十行 FFI（`internal/os.rs` 已有 FFI 豁免先例） |
| Defender | 无现成实现；两条路：**(i) 纯 Rust**：`QueryServiceStatusEx` 查 WinDefend 服务 + 读 `HKLM\SOFTWARE\Microsoft\Windows Defender\Real-Time Protection\DisableRealtimeMonitoring` 与 `...\Exclusions\Paths`；**(ii) 走 pwsh `Get-MpPreference`**（`internal::os::ps_command()` 已具备，`os.rs:194`） | ⚠️ 最重的一项，见 §4.3 |

### 4.3 Defender 检查的实现选择（实现期子决策）
- **(i) 纯 Rust（推荐）**：无 pwsh 冷启动（~0.5–1s），符合"更快"；风险：Tamper Protection 可能限制注册表读取，且注册表布局非公开契约（官方用 `Get-MpPreference` 是权威来源）。
- **(ii) pwsh `Get-MpPreference`**：与官方判定逐字一致；代价是依赖 pwsh/powershell 存在且每次多一次进程启动。
- **折中**：优先纯 Rust；读不到 Defender 配置时降级到 pwsh（或标记为"无法判定"而非误报）。
- 官方语义细节：Defender 仅在管理员下检查两项（local/global 各一次），WDAGUtilityAccount 跳过。

### 4.4 测试与验证
- 环境检查的**判定逻辑**做成纯函数（输入 = admin/bucket 列表/注册表值/helper 状态/卷格式，输出 = 问题列表），单测覆盖各分支（离线、确定性）；OS 读取端做薄封装。
- e2e：worktree 内 `cargo build` → 复制二进制到隔离路径 → 用临时配置目录跑 `hok checkup` 与 `hok health`，核对输出与退出码；不改动本机 Scoop 状态。
- 既有受影响的测试：`tests/cli_help_i18n.rs`（help 文案）、`tests/i18n_keys.rs`（键对齐）、`tests/exit_codes.rs`（若加断言）。

### 4.5 退出码（子决策）
官方 `checkup` **始终 exit 0**（即使有 issues）。两个选项：
- **与官方一致 = 0**：符合 S1 的"逐命令对齐官方退出码"口径；
- **hok 语义 = 有问题返回 1**：脚本更友好，但偏离官方。
建议：本会话对齐官方（0），在文档登记；若你希望"更好"，改为 1 需另行拍板。

---

## 5. 影响面汇总
- 代码：`src/cmd/{checkup,mod}.rs`、新增 `src/cmd/health.rs`、libscoop 新增环境诊断模块（+ 可能 `internal/os.rs` 补 FFI）、`package/mod.rs`、`package/resolve.rs`（helper 可见性）。
- 文案：`locales/en.yml`、`locales/zh.yml`。
- 文档：`README.md` / `README-zh.md` 命令列表、`AGENTS.md` 命令清单、`.agents/skills/scoop-gap-audit.md`、本计划 §6、审计报告 §2 状态行、`CHANGELOG`（如惯例）。
- 技能/记忆中的命令清单（AGENTS.md 的 Commands list）需加 `health`。

---

## 6. 待你拍板
1. **语义方案**：A（替换，删体检）/ **B&C 并存（推荐）** / D（不做，登记偏离）。
2. **退出码**：**与官方一致恒 0（推荐）** / 有问题返回 1。
3. （实现期，若未反对则按推荐执行）Defender 用**纯 Rust + 读不到时降级**。
