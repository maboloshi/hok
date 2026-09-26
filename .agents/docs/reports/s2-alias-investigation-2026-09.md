# S2 `alias` 可用性：调研报告（2026-09-12）

> **范围**：`.agents/docs/active/plan/command-deviation-fix-plan-2026-09.md` §S2；报告起点 `.agents/docs/active/audit/command-deviation-audit-2026-09.md` §6.1（7 条）。
> **纪律**：本报告只做调研，**未改任何代码**。等拍板后再实现。
> **工作区**：worktree `.worktrees/s2-alias`，分支 `fix/alias-availability`，基线 `3d4969f`（`refactor/edition-2024`）。
> 注：`.agents/docs/`、`.agents/skills/`、`.worktrees/` 均为可见但未跟踪目录（`.dsh` 其余内容被忽略），worktree 内不含报告与技能文件；收尾的文档同步仍回主工作树做。

---

## 1. 复现（实测）

环境：`target/release/hok.exe`（2026-09-12 22:00 构建），用隔离 `HOME`/`USERPROFILE=%TEMP%\hok-s2-repro` 运行，**未触碰真实 `~/.config/hok/config.json`**。

| # | 命令 | 实际输出 | 退出码 |
|---|---|---|---|
| 1 | `hok alias add foo 'hok list'` | `别名已添加: foo -> hok list` | 0 |
| 2 | — | 配置文件内容 `{"alias":{"foo":"hok list"}}` | — |
| 3 | `hok alias list` | `:: 别名` / `foo  hok list` | 0 |
| 4 | **`hok foo`** | `error: unrecognized subcommand 'foo'` + Usage | **2** |
| 5 | `hok alias add foo 'hok search x'`（重名） | `别名已添加: foo -> hok search x`，旧值被静默覆盖 | 0 |
| 6 | `hok alias rm nope`（不存在） | `别名已删除: nope`（**误报成功**） | 0 |
| 7 | `hok alias`（裸） | 等同 `list` | 0 |
| 8 | `hok alias list -v` | 无 Summary 列（`-v` 被全局 verbosity 吃掉，不报错） | 0 |
| 9 | `hok alias add list 'hok search x'` | 接受；`hok list` 仍走内建命令，别名**永不可达** | 0 |
| 10 | `hok alias add 'a b' 'hok list'` | 接受（名字无任何校验） | 0 |

**核心缺陷（#4）坐实**：`alias add` 写入的别名无法作为命令调用，`alias` 命令实质上只是"配置里的一段死记录"。

**报告更正一处**：§6.1 第 7 行写「hok 打印错误但返回 Ok → exit 0」。实测（#6）不是"打印错误"，而是**完全静默成功并打印"已删除"**——语义比报告描述的更差。

---

## 2. 证据（源码对照）

### 2.1 hok：只写不读

- 写入：`crates/libscoop/src/config.rs:1300-1306`（`alias_add`）→ `set_alias`（`config.rs:423-437`）→ `config::write`。
- 存储语义：**`名字 → 命令行字符串`**（实测 #2）。
- 读取：全仓唯一消费点是 `src/cmd/alias.rs:48`（`config.aliases()`，仅用于列表）。
  `git grep` 结果：`aliases()` 仅 1 处调用；`set_alias` / `alias_add` / `alias_remove` 无其它调用方。
- 分发：`src/cmd/mod.rs:340-350` — `Cli::completions`… 实际为 `Cli::from_arg_matches(&cmd.get_matches_from(normalized))`；`get_matches_from` 对未知子命令**直接 `exit(2)`**（#4），根本不进 `start()` 的 dispatch 分支。

### 2.2 官方 Scoop：脚本 + 索引，双份

- `lib/commands.ps1:43-74` `add_alias`：
  - 写 `<shims>\scoop-<name>.ps1`，内容 = `# Summary: <描述>` + 换行 + **命令正文**；
  - 在 config 的 `alias` 里记 `名字 → "scoop-<名字>"`（**脚本名，不是命令正文**）；
  - 重名 → `abort "Alias '<name>' already exists."`；脚本文件已存在 → 另有一条 abort。
- `lib/commands.ps1:84-86` `rm_alias`：不存在 → `abort "Alias '<name>' doesn't exist."`；存在则删脚本 + 删 config 项，并先打 `Removing alias '<name>'...`。
- `lib/commands.ps1:96-127` `list_aliases`：读脚本文件 → `Command`（第 2 行起）与 `Summary`（首行 `# Summary:`）；**文件缺失标记 `<BROKEN>`**；`-v` 追加 Summary 列；空则 `No alias found.`。
- 调用链：`bin/scoop.ps1:44`（`commands` = libexec + `shims\scoop-*.ps1`）→ `exec`（`bin/scoop.ps1:52`）→ `command_path`（`lib/commands.ps1:19-32`，libexec 优先，否则取 shims 脚本）→ `& $cmd_path @arguments`；脚本内 `$args` = 别名之后的参数。
- 退出码：`libexec/scoop-alias.ps1:32-40` 裸/未知子命令 → `error` + `my_usage` + `exit 1`；`abort`（`lib/core.ps1:293`）默认 `exit 1`。
- `libexec/scoop-alias.ps1:9,45` 支持 `alias add <name> <command> [<description>]`。

**结论**：hok 只搬了官方"config 索引"这一半，而且索引值的语义刚好相反（官方存脚本名，hok 存命令正文）；真正让别名可执行的另一半（脚本）完全缺失。

### 2.3 迁移路径的连带影响

`crates/libscoop/src/config.rs:92-123`：hok 首次启动会把 Scoop config 中**已建模的键**抄进 hok config；`alias` 在建模内（`config.rs:157`）。
→ 从 Scoop 迁移过来的用户，hok config 里是 `{名字: "scoop-<名字>"}`（脚本名）。今天 `hok alias list` 会把这个脚本名当"命令行"原样显示（误导），选 A 方案则正好能直接接上（官方脚本文件仍在 shims 里）。

---

## 3. 候选方案与代价

### A) 官方式脚本别名（写 `shims\scoop-<name>.ps1` + config 存脚本名）

- 做法：`alias add/rm` 落盘/删脚本；分发层（未知子命令）查 config → 取 shims 里的脚本 → pwsh 执行并透传 `$args`；`list` 读脚本显示 Command/Summary/`<BROKEN>`。
- 收益：**唯一能做到与官方双向互通的方案**（迁移来的 Scoop 别名可直接用；hok 建的别名官方 Scoop 也能用；description/`-v`/`<BROKEN>` 全部自然成立）。
- 代价与风险：
  1. **与 hok shim 子系统耦合**：`crates/libscoop/src/package/shim.rs:22-35` 的 `list_shims` 会把 `<shims>\*.ps1` 收进 `hok shim list`；`shim.rs` 的 remove/alter/备份-还原逻辑也会扫同一目录。别名脚本必须在这套生命周期里被排除，否则 `hok shim list` 多出幽灵项、`shim rm` 可能删掉别名脚本。这是**实打实的耦合成本**，且属于"给别的流程打补丁"的味道。
  2. 破坏性：hok 现有 `alias` 值语义（命令正文）要改成脚本名，已存在的 hok config 别名需要迁移/兼容。
  3. 运行时仍需 pwsh（与 B1 相同）。
- 规模：约 200 行 + shim 子系统排除 + 文件生命周期测试。

### B) 内部分发（不落盘，config 保持 `名字 → 正文`）

在未知子命令的错误路径上拦截（`try_get_matches_from` 失败且是未知子命令 → 查 config 别名 → 执行），happy path 完全不变。正文语言有两种选择：

- **B1 正文是 PowerShell，用 pwsh 执行**（官方正文语义，支持 `$args`）
  - 做法：`pwsh -NoProfile -ExecutionPolicy Bypass -Command "<正文>"`，别名后的参数作为 `$args` 传入；stdout/stderr 直通；子进程退出码透传（走现成的 `crate::ExitCode`，`src/lib.rs:63`）。可复用 `internal::os::ps_command()`（`crates/libscoop/src/internal/os.rs:194`）。
  - 收益：最小改动即可让 hok 自己的别名真正可用；`hok alias add rm 'hok uninstall $args[0]'` 与官方用法一致；无新落盘产物、零 shim 子系统耦合。
  - 代价：官方已有别名（值是 `scoop-xxx` 脚本名）跑不通；无 description / `<BROKEN>` / `-v` Summary；每次调用有 pwsh 冷启动（~0.7s，hok 安装脚本本来也依赖 pwsh）。
  - 规模：约 80 行 + 测试。
- **B2 正文是 hok 命令行，进程内重分发**（`hok alias add rm uninstall` → `hok rm x` = `hok uninstall x`）
  - 收益：最快、无 pwsh、纯原生、与 hok 单二进制气质最合；参数直接追加，无引号/转义地狱。
  - 代价：**与官方 alias 正文语义（PowerShell）不同**，官方写法 `scoop uninstall $args[0]` 不成立；迁移来的别名（脚本名）同样无效。属"定义 hok 自有语义 + 文档化"路线。
  - 规模：约 60 行 + 测试。

### C) 只文档化（明确"仅记录"语义）

- 做法：`--help`/README 写明"别名仅记录、不可调用"，或把展示名改成"别名记录"。
- 收益：零成本、零风险。
- 代价：报告 §6.1 的核心偏离原样保留；`alias` 这个交互式命令继续是"看起来能用、实际不能用"的陷阱。

### 代价对比

| | 官方互通 | 需 pwsh | 落盘产物 | shim 子系统耦合 | 破坏现有 config | 规模 |
|---|---|---|---|---|---|---|
| A | ✅ 双向 | ✅ | ✅ | ⚠️ 需排除逻辑 | ⚠️ | ~200 行 |
| B1 | ❌ | ✅ | ❌ | ❌ | ❌ | ~80 行 |
| B2 | ❌ | ❌ | ❌ | ❌ | ❌ | ~60 行 |
| C | ❌ | ❌ | ❌ | ❌ | ❌ | ~10 行 |

**推荐：B1**（次选 A）。理由：`alias` 的核心缺陷是"不可调用"，B1 用最小代价把它修好且不引入跨子系统耦合；官方正文本就是 PowerShell，B1 保持正文语义一致，只是不采用官方的脚本落盘实现（hok 是单二进制，shims 目录归包管理生命周期所有）。若你更看重与官方 Scoop 的**双向互通**（迁移用户 + 共享 shims），选 A——但要接受 shim 子系统的排除成本和 config 语义变更。

---

## 4. §6.1 其余 6 行的处置建议（与主方案解耦，可一并拍板）

| 报告行 | 现状（实测/源码） | 建议 |
|---|---|---|
| 裸 `alias` 默认 list（官方 exit 1） | #7 | **保留 hok 便利语义**，登记为有意偏离（低成本、无脚本兼容风险） |
| `alias add` 无 description | `src/cmd/alias.rs:28-33` | 选 A 则天然支持；选 B 则**不做**并文档化（config 里没有描述的位置） |
| `alias list` 无 `-v` Summary | #8 | 同上：选 A 时做；选 B 不做 |
| 截断 60 字符 / 不识别脚本缺失 | `src/cmd/alias.rs:55-60` | 选 B 时**去掉截断**（官方输出完整命令，改动 3 行）；`<BROKEN>` 概念在 B 下不存在 |
| 重名 `add` 静默覆盖（官方 abort exit 1） | #5 | **对齐**：报错 + exit 1（B 下同样适用） |
| `rm` 不存在静默成功（官方 abort exit 1） | #6 | **对齐**：报错 + exit 1（B 下同样适用） |
| 额外发现：别名名与内建命令重名 → 永不可达 | #9 | **新增校验**：`alias add` 拒绝内建命令名（官方靠 libexec 优先静默遮蔽，hok 显式报错更安全）；名字含空白/路径分隔符也一并拒绝 |

---

## 5. 影响面与提交拆分（按推荐 B1 预估）

**影响面**

- `src/cmd/mod.rs`：`start()` 中改为 `try_get_matches_from`，未知子命令错误路径上查别名并分发（happy path 不变）。
- `src/cmd/alias.rs`：入口/分发执行（pwsh 调用、`$args`、退出码透传）、重名与不存在报错、去截断、名字校验。
- `crates/libscoop/src/config.rs`：`alias_add`/`alias_remove` 返回"已存在/不存在"错误（或加 `alias_contains`）；不动存储格式。
- `locales/en.yml`、`locales/zh.yml`：新增文案。
- `README.md` / `AGENTS.md` 命令说明（alias 用法示例）。
- 测试：`src/cmd/alias.rs` / `config.rs` 单测；pwsh 可用时跑一次端到端（无 pwsh 则跳过，沿用 `script.rs` 的 `pwsh_available()` 模式）。

**提交拆分**（一主题一 commit）

1. `feat(alias): dispatch unknown subcommands through configured aliases`（核心可用性 + 最小测试）
2. `fix(alias): reject duplicate/reserved names and missing removals with exit 1`（对齐官方 abort 边界）
3. `docs(alias): document alias semantics`（README/locales；若含去截断等输出对齐，可并入 2 或单独一条按体量定）

---

## 6. 需要你拍板的点

1. **主方案**：A（官方式脚本别名，双向互通，需处理 shim 子系统耦合）／B1（config + pwsh 执行正文，推荐）／B2（config + 原生参数重分发）／C（只文档化）。
2. **边界行**：重名 add、不存在 rm 是否一并对齐成"报错 + exit 1"（建议是）；`alias add` 名与内建命令重名是否拒绝（建议是）。
3. **裸 `alias` 默认 list**、**去 60 字符截断**是否一并做（建议：裸 alias 保留现状并登记有意偏离；去截断随主方案一起做）。
4. 若选 A：是否接受 config 里 `alias` 值从"命令正文"改为"脚本名"（破坏性变更，需迁移已存在的 hok config）。

---

## 7. 决策与落地（2026-09-13）

**决策**：选 **A（官方式脚本别名）**；四个附带项全选——重名 add / 不存在 rm 报错 exit 1、拒绝与内建命令重名、去掉 60 字符截断、裸 `alias` 保留 list 并登记有意偏离。

**落地**：worktree `.worktrees/s2-alias`，分支 `fix/alias-availability`，提交 **`3c90095`**
`feat(alias): run aliases as upstream-style shim scripts`（签名已验证）。
合并到主线时把该分支合回 `refactor/edition-2024`（或先 rebase）。

**实现要点**

- 新增 `crates/libscoop/src/alias.rs`：alias 生命周期的**唯一入口**（`script_path`/`resolve`/`list`/`add`/`remove`/`run`），CLI 不再自己碰 shims 目录或 `alias` 配置键；原 `config::alias_add`/`alias_remove`（只写 config）删除。
- 脚本内容与官方**逐字节一致**：`# Summary: <desc>\n<command>\r\n`，UTF-8 无 BOM（`lib/commands.ps1:65-71` + `Out-UTF8File` 的 `WriteAllLines`）。
- config 存 `名字 -> "scoop-<名字>"`（脚本名），与官方一致；`list` 从脚本读回 Command/Summary，脚本丢失显示 `<BROKEN>`。
- 分发在 clap `InvalidSubcommand` 错误路径上拦截（happy path 不变）：`<shims>\scoop-<token>.ps1` 存在即执行，`pwsh -File <script> <args...>`、stdio 继承、退出码透传；非别名一律回落 clap 原诊断（信息与 exit 2 不变，已与旧二进制逐项比对）。
- 新增错误变体 `AliasAlreadyExists`/`AliasNotFound`/`AliasScriptExists`/`AliasInvalidName`/`AliasReservedName` + en/zh 文案，经 `translate_error` 本地化。

**调研预估的修正**

1. **shim 子系统耦合比预估轻得多**：官方 `scoop shim list` 本身就 `Get-ChildItem -Include '*.shim','*.ps1'`（`libexec/scoop-shim.ps1`），所以别名脚本出现在 `hok shim list`（实测显示 `scoop-greet`）是**对齐行为**，不需要任何排除逻辑——方案 A 的主要"代价"随之消失。
2. **分发只查文件、不查 config**：官方 `commands`/`command_path` 完全按 `shims\scoop-*.ps1` 解析，config 只作索引与 `list` 的来源；照此实现后，`<BROKEN>` 条目自然不可调用（与官方一致）。
3. **报告更正**：§6.1 第 7 行「hok 打印错误但返回 Ok」不准确——改动前 `alias rm nope` 是**完全静默成功**并打印"别名已删除"、exit 0（实测）。

**本仓特例（新增，非偏离）**

- `rm`/`s`/`u`/`remove` 是 hok 的 clap 顶层别名，因此官方文档示例 `scoop alias add rm 'scoop uninstall $args[0]'` 在 hok 会被拒绝（hok 本就有 `hok rm`）。这是"拒绝内建重名"的直接结果，属于有意为之。

**仍未做（有意偏离 / 待办）**

| 项 | 现状 | 处置 |
|---|---|---|
| 裸 `alias` 默认 list | 保留（官方要求子命令并 exit 1） | **有意偏离**，已登记 |
| 空表文案 | `No aliases configured.`（官方 `No alias found.`） | 不列为 S2 范围，待 S7 文案批量 |
| `alias rm` 成功文案 | `alias removed: x`（官方先打 `Removing alias 'x'...`） | 未列行，保持 hok 自有文案 |
| `hok help <alias>` | 不渲染别名帮助（官方 `scoop help <alias>` 会） | 未列行，登记为已知差异 |
| `alias list -v` 的实现 | 复用全局 verbosity（与 `info -v` 同一约定，见计划 §4"不做清单"） | **有意偏离**，已登记 |
| 别名自递归（body 调用自身） | 无深度保护（官方同样没有） | 已知限制 |

**验证**

- 单测：`libscoop::alias` 12 项（脚本字节、config 索引、BROKEN、重名/非法名/占用脚本、rm 幂等与容错、解析函数）、`cmd::alias` 3 项（表格列宽/无尾随空格/Summary 列/BROKEN）、`i18n_keys` 3 项。
- E2E（隔离 `HOME`/`SCOOP`，debug 二进制）：add（含描述）→ 脚本字节核对 → `list`/`list -v` → `hok greet one two` → `--lang zh greet a` → 退出码透传（`exit 3` → 3）→ 重名/不存在/内建重名/非法名均 exit 1 → 未知 token 仍 exit 2 → 删脚本后 `list` 显示 `<BROKEN>` 且调用回落 exit 2 → `rm` 清文件与 config。
- 与旧 release 二进制逐项比对 `hok`（无参）、`hok help x`、`hok nope`、`hok --help` 的退出码：全部一致。
- 确定性门：改动的 7 个文件 `rustfmt --edition 2024 --check`（`skip_children=true`）通过；`cargo clippy --workspace --all-targets` 对本次文件**零告警**（仓库既有告警全部来自未改动的 vendored `innospect`）。

