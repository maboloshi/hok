# hyperscoop 借鉴项落地计划（6 + 1 项）

Status: active
Updated: 2026-10-07

> **来源**：调研 [`research/hyperscoop-feature-survey-2026-10.md`](../../research/hyperscoop-feature-survey-2026-10.md)
> （§0 依赖面实测、§2 命令面四种性质、§4 反例）。本文件是**执行件**。
> **用户决策（2026-10-07）**：6 项全做；`update -q/--quiet` 同批；`service` **只做命令**（卸载不自动停服务）；
> **不加 `doctor` 别名**（保持 `checkup`/`health` 两名）。
> **口径**：只做"纯新增命令/新增开关"，**不改同名命令的既有语义**（调研 §2）；官方 Scoop 是行为锚点。

## 0. 通用约束（每项都适用）

- **官方锚点**：改同名命令前先读 `../Scoop/libexec|lib`；新增命令不受官方约束，但**不得**顺手改同名命令行为。
- **i18n**：所有新文案进 `locales/en.yml` + `locales/zh.yml`，键名沿用现有 `cmd.*` 风格；`hok-i18n` 是构建期表，
  新增键必须两边同步（有键对齐测试）。
- **分层**：CLI 层只做参数/渲染/调度（[[hok-code-organization]]）；库层不写 stdout，走 `Session::output()` / 事件。
- **新 FFI**：`unsafe_code = deny` + 逐点豁免 ⇒ 新增 `extern` 必须同步登记
  [`active/audit/unsafe-audit.md`](../audit/unsafe-audit.md)。
- **提交**：一项一提交（代码 + 该主题文档回填）；devlog 条目自成收尾提交（引用该主题 sha）。
- **门禁**：`cargo test`、文件级 `rustfmt --edition 2024 --check`（不带 `skip_children`）、
  `cargo clippy --workspace --all-targets -- -D warnings`（见 [[hok-rust-gate-practice]]）。

## 总表（顺序 = 落地顺序，按性价比）

| # | 项 | 命令面变化 | 主要文件 | 成本 |
|---|---|---|---|---|
| 1 | `health` 补两项检查 | 无（同名命令，属 hok 独有命令的加固） | `crates/libscoop/src/package/health.rs`、`package/shim.rs` | 小 |
| 2 | `edit <app>` | 新增命令 | `crates/libscoop/src/config.rs`、`package/manifest_source.rs`、`src/cmd/edit.rs` | 小 |
| 3 | `--dry-run` | 新增开关（uninstall/cleanup → install） | `package/sync_remove.rs`、`package/cleanup.rs`、`package/sync.rs`、`src/cmd/*` | 中 |
| 4 | `update/upgrade -c` | 新增开关 | `src/cmd/update.rs`、`src/cmd/upgrade.rs`（调 `package::cleanup::cleanup`） | 小 |
| 5 | `reset` 对齐官方 | 同名命令补分支（**官方锚点项**） | `src/cmd/reset.rs`、`package/sync_remove.rs` | 中 |
| 6 | `service` 原生 | 新增命令 | `crates/libscoop/src/internal/os.rs`、`package/service.rs`、`src/cmd/service.rs` | 大 |
| 7 | `update -q/--quiet` | 新增开关（**官方锚点项**） | `src/cmd/update.rs`、`src/cmd/upgrade.rs` | 小 |

---

## 1. `health` 补：断链 shim + `current` 悬空

**为什么放 `health` 而不是 `checkup`**：官方 `checkup`（`libexec/scoop-checkup.ps1:11-47`）只有 9 条环境判定，
**不含** shim/junction 检查；hok `checkup` 已对齐官方（`diagnostic.rs:193-261`）⇒ 加进去即静默偏离官方基线。
`health` 是 hok 独有命令，语义就是"已装包的问题"（`health.rs:51` 的 install.json / current / manifest.json），
这两条正是它的缺失分支。

- **新增分支**：`current` 存在但**目标失效**（悬空 junction/符号链接）→ 报 `cmd.health_<key>`，建议 `hok reset <app>`。
  现状只覆盖"没有 current"（`health.rs:95`），不覆盖"current 在、目标没了"。
- **新增分支**：shim 目标缺失 → 扫 `session.shims_dir()` 下每个 shim，用 `package::shim::shim_target()`
  （`shim.rs:105`，覆盖 `.shim`/`.cmd`/`.ps1`，目标不存在即 `None`）判定，报 shim 名 + 失效目标。
- **保持只读**：不做 `--fix`（与 `checkup` 一致）；修法在文案里给命令。
- **测试**：`health.rs` 现有 `#[cfg(test)]`（junction/no-junction 两态）扩两条——悬空 current、失效 shim；
  含"目标存在时不报"的反例。
- **验收**：`hok health` 在健康环境输出不变（回归）；构造一个悬空 current 与一个假 shim，两条都能报出。

## 2. `edit <app>`

- **复用**：`libscoop::config::edit(session)`（`config.rs:1462-1491`）已实现"config `editor` → `$EDITOR` +
  `cmd /c` 回退（`.cmd`/`.bat`/带参数编辑器）"，但硬编码打开 config 文件 ⇒ 抽出
  `open_in_editor(session, path)`，`config::edit` 改为传 config 路径。**不新增配置键**（`editor` 已存在）。
- **打开对象**：**bucket 里的 manifest**（与官方 `cat`/`home` 同源），不是已安装副本。
- **路径来源**：`GetManifest`（`manifest_source.rs:46`）目前没有文件路径字段 ⇒ 加 `pub file: Option<PathBuf>`，
  在其构造点（`:161/:264/:337/:466/:522`）按来源填；URL/未落盘来源为 `None`。
- **查询解析**：与 `cat` 共用 `resolve_manifest_query`（`src/cmd/cat.rs:85`），多 bucket 时沿用现有多选交互。
- **边界**：来源是 URL 或已装但无 bucket 记录 ⇒ 明确报错"没有可编辑的本地 manifest"（不静默下载临时文件）。
- **不加 `-e/--editor`**：hyperscoop 有，官方无；已有 `editor` 配置 + `$EDITOR` 两条路，够用。
- **验收**：`hok edit <app>` 打开 bucket manifest；`editor` 设为 `"code --wait"` 这类带空格串仍可用（走 `cmd /c` 回退）；
  URL 查询报错不退化为 panic。

## 3. `--dry-run`（uninstall → cleanup → install）

- **原则**：计划必须由**将要执行的那条 resolve 路径**产出，禁止另写一份"演示用"解析（否则计划会撒谎）。
- **接入点**：
  - uninstall：`package::sync_remove`（现有 `check_not_running` 门控在 `sync_remove.rs:237`）加只读计划分支，
    打印将卸载的包、作用域、是否 purge、将删的 persist 项、将删的 shim/快捷方式。
  - cleanup：`package::cleanup::cleanup`（`cleanup.rs:286`）已有 Plan 阶段（`:299-350`）⇒ 暴露 `plan_only`
    （或只传不落地的 observer）即可，零重复实现。
  - install：`package::sync::sync`（`sync.rs:424`）加 `SyncOption::DryRun`，打印解析出的包 + 依赖 + 架构 +
    将生成的 shim/快捷方式。
- **零副作用契约（硬性）**：不弹 UAC / 不提权、不联网、不更新 bucket、不写 config（含 `last_update`）、
  不下载、不写缓存、不建/删文件、不提示交互。**这是 hyperscoop dry-run 的反面**——它在 global 时仍
  请求管理员、`-u` 仍更新 bucket 并写 config（`invoke_install.rs:18-27,62`）。
- **命名**：`-n/--dry-run`（install 的 `-d` 是 download-only，帮助文案里与之区分）。
- **测试**：三个命令各一条"演练后临时根目录逐文件快照不变"的测试；加一条"演练仍解析依赖"的断言。
- **验收**：`hok uninstall -n <app>` / `hok cleanup -n` / `hok install -n <app>` 输出计划；用文件快照断言零写入。

## 4. `update/upgrade -c/--cleanup`

- **不做 hyperscoop 那种实现**：它删掉 `current` 指向的版本**再安装**（`crates/command/src/update.rs:81-102,195-201`）
  ⇒ 安装期间 `current` 悬空、无锁、且 `current` 已被 `switch` 到最新版时会**删掉最新版**。
- **正确形态**：升级**成功之后**，对本次实际升级成功的包调用既有
  `package::cleanup::cleanup(session, &names, &CleanupOptions, &CleanupObserver)`
  （`cleanup.rs:286`，已并行 + 逐版本进度），保留 current、删旧版本；**不碰缓存**（缓存是 `-k` 的事）。
- **语义边界（需在帮助文案写清）**：只清理"本次升级过的包"；无包升级（如仅更新 bucket、或 `-U`）时无操作，
  提示可跑 `hok cleanup -a`。
- **验收**：升级一个有多版本残留的包后 `-c` 生效；只更新 bucket 时 `-c` 无副作用且给出提示。

## 5. `reset` 对齐官方（"switch 需求"的正解）

官方 `scoop reset`（`libexec/scoop-reset.ps1:16,28,31,33-36,48-50` + `lib/core.ps1:1234`）：

- 支持 `bucket/app` 与 `app@version`（`parse_app` 正则）；
- 支持**多应用**与 `*`/`-a`（`:28` 逐应用 error+continue）；
- **无 `-g/--global`**：未显式指定时，若该 app 已全局安装则自动置 global（`:33-36`）；
- 未给版本 ⇒ 回退 `Select-CurrentVersion`。

hok 现状（`src/cmd/reset.rs:10-18,71-75`）：`<app> [version]` 位置参数、`-a`；**不支持** `app@version`/`bucket/app`、
多应用、全局自动识别（既有登记差异）。

- **做**：`app@version` / `bucket/app` 解析（复用 `manifest_source::resolve_isolated_query` 的既有分派，
  **不要**再写一套），多应用循环（失败继续、编号连续），全局自动识别。
- **不做**：不加 `switch` 命令 —— 它的两个能力已分别由 `reset <app> <ver>` 与 `hok list -V/--versions`
  （`list.rs:40,142-163`，`is_current` 标出当前版本）覆盖，再加就是同一件事的第三个名字（调研 §2 的 (b) 类）。
- **验收**：`reset app@1.2.3`、`reset bucket/app`、`reset a b c`、全局已装自动识别四条各有测试；
  `-a` 行为与官方一致（失败继续）。

## 6. `service`（原生 SCM）

- **原生，不用 `sc.exe`**：`internal/os.rs:554-611` 已有 `#[link(name = "advapi32")]` +
  `OpenSCManagerW/OpenServiceW/QueryServiceStatus/CloseServiceHandle`；补齐
  `EnumServicesStatusExW`（枚举 name/display/state）、`QueryServiceConfigW`（ImagePath）、
  `StartServiceW`、`ControlService`（STOP）。
- **归属判定**：取 ImagePath（去引号与参数）→ 与 `<apps_dir>\<name>\` 前缀匹配，复用
  `starts_with_ignore_case`（`os.rs:234`）。**不新增 manifest 字段**（官方 schema 无 service；
  造字段是更大的偏离，见调研 §4）。
- **命令面**：`hok service`（列出属于已装包的服务）、`hok service status|start|stop|restart <app|service>`。
  `restart` = stop → 轮询 `QueryServiceStatus` 直到 STOPPED（超时上限）→ start，
  **不用** hyperscoop 的固定 `sleep 800ms`（`invoke_service.rs:182-187`）。
- **权限**：start/stop 前查 `session.is_admin()`，不足则明确报错（hyperscoop 不检查，直接吞 `sc.exe` 输出）。
- **与卸载的关系（本次不做自动停）**：hok 的 `check_not_running` 会因 app 目录下有进程而跳过/中止
  （`sync_install.rs:498`、`sync_remove.rs:237,537`），服务会自我重启 ⇒ 仅**在 running-process 警告文案里
  点名服务并提示 `hok service stop <app>`**；自动停服务属偏离官方，单独决策。
- **FFI 登记**：新增 extern 进 `unsafe-audit.md`。
- **验收**：本机实测样本 = `Everything (1.5a)`（`apps\everything-alpha\current\Everything.exe -svc`）与
  `ToDesk_Service`（`apps\todesk\current\ToDesk.exe --runservice`）；非管理员下 start/stop 给出明确错误；
  枚举不依赖 `sc.exe`（断言无子进程）。

## 7. `update -q/--quiet`（官方锚点缺口）

官方：`scoop-update.ps1:8-15,38,261,288,293` ⇒ `-q/--quiet` 抑制**包升级路径**的信息输出
（如"latest version already installed"警告、nightly 版本获取提示），错误仍照常输出。

- **做**：hok `update`/`upgrade` 补 `-q/--quiet`（hok 全仓当前无 `-q`），只抑制信息级输出（"已是最新"、
  bucket 同步进度之外的提示），错误/警告/最终汇总不动。
- **不做**：不改 bucket 同步的输出契约（官方 `-q` 也到不了 `Sync-Bucket`）。
- **验收**：`hok update <app> -q` 在"已是最新"时无输出、退出码不变；错误路径仍打印。

---

## 8. 收尾与验收

每项落地后：`cargo test` 全绿 + 文件级 fmt/clippy 干净 + 该主题文档回填（本计划的状态、调研的相关小节）
+ 一条 devlog；全部完成后本文件归档到 `reports/`（R4/R5）。
