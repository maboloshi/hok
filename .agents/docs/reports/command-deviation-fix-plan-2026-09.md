# hok 偏离修复：新会话拆分计划（2026-09-12）

> **来源**：`.agents/docs/active/audit/command-deviation-audit-2026-09.md`（约 280 条偏离，双侧 file:line）。
> **目的**：把修复工作切成"一条会话 = 一个主题"的独立单元，每条新会话自带范围、决策门、验收与提交拆分。
> **纪律**：每条会话都必须过 全局 `investigate-first`（动手前实测报告 + 选项 + 等确认）、全局 `proactive-review`（提交前 review）、`no-patchwork-fixes`、全局 `rust-deterministic-gate` + `hok-rust-gate-practice`（本仓 review 对照项 `hok-proactive-review`、并行细则 `hok-worktree-parallel`）。
> **状态**：当前有效，供逐条开工。

---

## 0. 使用方式

**新会话开场模板（复制一行即可）**：

```
按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S<n> 执行：先调研出报告（期望 vs 实际、证据、候选方案与代价、影响面），等我确认后再改。
```

**每条会话的固定收尾**：

1. 更新本计划 §6 进度勾选 + 备注（提交号 / 决策结论）；
2. 报告 `.agents/docs/active/audit/command-deviation-audit-2026-09.md` 对应行标注状态（已修 / 有意偏离 / 暂缓）；
3. 同步 `.agents/skills/scoop-gap-audit.md`（该技能的 ✅/⏸ 标记 + 新结论）。

**重要前提**：报告是**候选清单，不是待办清单**。预计最终"值得做"的条目远少于 280 条——每条会话的任务之一就是**先判定这条该不该做**（对齐 / 定义 hok 语义 + 文档化 / 不做）。

---

## 1. 会话总览

| 会话 | 标题 | 报告起点 | 规模 | 前置 | 决策门（需要你拍板） |
|---|---|---|---|---|---|
| **S1** | 退出码与批量失败策略口径 | §1、§4.4、§4.5、§5、§7 | 中 | 无 | 是否放弃"全局 0/1"改用官方分命令退出码 |
| **S2** | `alias` 可用性 | §6.1 | 中 | 无 | 生成可执行别名（shim/内部分发）vs 明确记录语义并在帮助里写明 |
| **S3** | `export` / `import` 与官方 Scoopfile 互通 | §6.4 | 中 | 无 | 全面采纳官方 schema vs 只做 import 兼容 vs 双格式 |
| **S4** | `config` 键策略与子命令语义 | §6.3 | 小-中 | 无 | 严格白名单 vs 宽容写入；`unset` 是否改名/兼容 `rm` |
| **S5** | `checkup` 语义决策 | §2、gap-analysis §4.4 | 中 | 无 | 对齐官方环境检查 vs hok 健康检查改名（如 `health`） |
| **S6** | `cleanup` / `cache` 数据安全 | §4.1、§4.2 | 小-中 | 无 | `-k` 是否改为保留当前版本缓存；`cache rm` 匹配改锚定 |
| **S7** | 低风险批量：文案 / 列名 / 列序 / 默认值 | §5、§6.2/§6.5、§7.6 | 小 | 无 | 一次性对齐范围（含测试快照改动） |
| **S8** | 路径解析类：`which` / `prefix` / `cat` / `home` | §5.6、§5.7 | 中 | 无 | `which` 是否输出 shim 指向的真实目标；是否解析 URL/UNC/本地清单 |
| **S9** | `checkver` 深度对齐（需再拆） | §7.1 | 大 | 无 | 范围取舍：52 条里哪些真值当 |
| **S10** | `checkhashes` 深度对齐（需再拆） | §7.2 | 中-大 | 无 | 缓存策略是否对齐官方（默认重下 vs 复用） |
| **S11** | 其余 hok 独有命令：`checkurls`/`missing-checkver`/`formatjson`/`ci-auto-pr` | §7.3–§7.5、§7.7 | 中 | 无 | 各自独立；`missing-checkver -s` 语义相反须先定 |
| **S12** | 插队小项：`ignore-failures` 默认显示疑似 bug | §8 | 极小 | 无 | 确认是否真 bug（核实后一次成型） |
| **S13** | 安装/卸载批量行为（可选，并入 S1 或独立） | §4.4、§4.5 | 中 | S1 | 运行中进程/未安装/held/被依赖 四类策略是否全部对齐 |
| **S14** | `bucket` / `shim` 参数与退出码（可选，并入 S7 或独立） | §6.2、§6.5 | 小-中 | 无 | `-g` 位置、`shim list *`、bucket 重复仓库检测是否对齐 |

**顺序建议**：S12 随时可插（最小）；S1 优先（它是 S13 与多处"退出码不同"的前置）；S2/S3/S5 是三个独立决策点，可任意先后；S9/S10/S11 属 hok 独有辅助命令，价值取决于你是否真在用它们。

---

## 2. 依赖关系

```
S12（插队，无依赖）
S1 退出码/失败策略口径 ──► S13 安装卸载批量行为
S2 alias ─┐
S3 export/import ─┤ 三条独立决策，互不影响
S5 checkup ─┘
S4 config ──► 后续若引入新配置键（update_nightly/hold_update_until 等）都落在这里
S6 cleanup/cache ──► 若做「缓存命中即校验」类优化，与本条合流（另见 devlog 开放项）
S7 低风险批量 ──► S14（可选合并：文案/退出码一次性改）
S8 路径解析类（独立）
S9/S10/S11 hok 独有命令深度对齐（互相独立，各自可再拆）
```

---

## 3. 各会话详案

### S1 退出码与批量失败策略口径

- **范围**：hok 全局 `0/1`（`src/bin/hok.rs:9`）+ clap 缺参一律 `exit 2`；官方各命令自定（0/1/2/3，`virustotal` 还有位掩码 0/1/2/4/8/16）。
- **决策门**：A) 引入按命令的退出码（对齐官方，脚本兼容性最好，但要逐命令梳理，兼容风险=依赖旧行为的脚本）；B) 保留 0/1，仅在明确处补特例（如 virustotal 位掩码）；C) 只对齐"缺参/未安装"这类高频场景。
- **调研要点**：先枚举官方每个命令的 exit code 出处（`libexec/*.ps1`），列出与 hok 的映射表；确认 `virustotal` 位掩码是否值得单独实现（报告 §7.5 已列）。
- **影响面**：`src/bin/hok.rs`、各 `src/cmd/*.rs`、`crates/libscoop/src/error.rs`（错误→退出码映射）。
- **提交拆分**：①错误→退出码映射基础设施（无行为变化）；②逐命令接入（每命令一个 commit 或按类分组）。
- **风险**：兼容性——用户脚本可能依赖 hok 现有 0/1；改动前需在计划里写明"破坏性变更"。

### S2 `alias` 可用性

- **范围**：报告 §6.1（7 条）。
- **核心问题**：hok 的 `alias_add` 只写 config 映射（`config.rs:1300-1306`），全仓无消费点（列表除外）⇒ **别名不可调用**。
- **候选方案**：
  - A) 生成 Scoop 式 `shims\scoop-<name>.ps1`（对齐官方；依赖 pwsh 可用；与 hok shim 体系需协调）；
  - B) hok 内部命令分发：wrapper 收到未知子命令时查 config 别名并执行（不落盘脚本，符合单二进制设计）；
  - C) 不做执行，只把"仅记录"语义写进 `--help`/README 并从"别名"改叫"别名记录"（最低成本，但功能仍缺）。
- **决策门**：A/B/C 选一；若选 A/B，还需定 `alias add` 的重名/`rm` 不存在时的退出码（对齐官方 abort）。
- **影响面**：`src/cmd/alias.rs`、`crates/libscoop/src/config.rs`、可能新增分发入口（`src/cmd/mod.rs` 或 `src/lib.rs`）、`locales/*.yml`、README。
- **提交拆分**：①语义/决策落地（可执行 or 文档化）；②重名/不存在退出码与文案对齐。

### S3 `export` / `import` 互通

- **范围**：报告 §6.4（10 条）。
- **核心问题**：schema 完全不兼容（hok `{buckets:{名:url}, apps:{bucket:{名:版本}}}` vs 官方数组 + `Info`）；hok `import` 期望 `apps` 为对象 ⇒ **官方 Scoopfile 导不进**。
- **候选方案**：A) 全面采纳官方 schema（破坏 hok 自有格式，但互通）；B) 只做 `import` 双向兼容（能读官方，也保留自己旧的），`export` 暂不动；C) `export` 支持 `--format scoop|hok` 双输出。
- **调研要点**：确认官方 `Info` 字段语义（`Global install` / 架构 / `Held package`）与 hok install 参数的映射；`-c/--config` 导出配置的键过滤规则。
- **影响面**：`crates/libscoop/src/package/{export,import}.rs`、`src/cmd/{export,import}.rs`、回归测试。
- **提交拆分**：①schema 决策落地（export 或 import 一侧）；②另一侧；③`-c/--config` 支持。

### S4 `config` 键策略与子命令语义

- **范围**：报告 §6.3（9 条）。
- **决策门**：①未知键：严格拒绝（现状）vs 官方式任意键写入（注意：官方无校验会导致写进无效键，hok 的严格是"更好但不同"——需你拍板）；②`unset` 是否改名 `rm` 或加别名；③是否补单键读取 `config <name>`；④`proxy none` 语义（删键 vs 写字符串 `none`）。
- **影响面**：`crates/libscoop/src/config.rs`（`set()` match、`list_all`、白名单测试 `unsupported_keys_rejected`）、`src/cmd/config.rs`、`locales/*.yml`。
- **注意**：`aria2-min-split-size` 默认 20M vs 5M 也在本条（一行改动，但要跑下载测试）。

### S5 `checkup` 语义决策

- **范围**：报告 §2；`.agents/docs/active/audit/scoop-gap-analysis-2026-08.md` §4.4。
- **候选方案**：A) 对齐官方环境检查（管理员/Defender/main bucket/长路径/开发者模式/7zip/innounp/dark/NTFS）；B) 保留 hok 健康检查并改名（如 `hok health`），`checkup` 腾给官方语义；C) 两者并存（`checkup`=环境检查、`health`=已装包健康）。
- **影响面**：新增/改名命令（`build.rs` 自动注册，删/加 `.rs` 即可）、`locales`、README/AGENTS.md 命令清单、技能记忆命令列表。
- **提醒**：改名会影响既有用户与文档引用，属破坏性变更，需在计划中标注。

### S6 `cleanup` / `cache` 数据安全

- **范围**：报告 §4.1、§4.2。
- **关键项**：`cleanup -k` 清空整个下载缓存（含当前版本，有重下代价）；`cleanup` 无参=清全部（官方报错）；`-g` 无 admin 检查；`-a -g` 只清全局；`cache rm` 子串匹配（`zip` 命中 `7zip`）；aria2 sidecar 不删。
- **提交拆分**：①`-k` 与 `cache rm` 匹配（数据安全，优先）；②`-a -g` / `-g` admin 检查；③无参语义与文案。

### S7 低风险批量：文案 / 列名 / 列序 / 默认值

- **范围**：`status`（§5.3）、`list`（§5.1）、`search`（§5.2）、`bucket`（§6.2）、`shim` 输出（§6.5）、`download --skip-hash-check` 主名与 `Cache is being ignored.`（§7.6）、`install/update` 的 `Everything is ok!`/文案（§4.3/§4.5）。
- **决策门**：一次要对齐到什么程度；是否接受改动现有测试快照与 i18n 文案（`locales/en.yml`、`zh.yml`）。
- **提交拆分**：按命令分组（一命令一 commit），避免巨无霸提交。
- **注意**：`aria2-min-split-size` 若并入本条，需单独验证下载路径。

### S8 路径解析类：`which` / `prefix` / `cat` / `home`

- **范围**：报告 §5.6、§5.7。
- **决策门**：`which` 是否输出 shim 指向的真实目标（需解析 hok shim 的目标记录）；是否支持 PATH 程序与全局 shim；`cat`/`home` 是否支持 URL/UNC/本地清单（对应官方 `Get-Manifest` 的完整语义）。
- **影响面**：`src/cmd/{which,prefix,cat,home}.rs`、`crates/libscoop/src/package/shim.rs`、可能新增 `Get-Manifest` 等价入口（**注意单入口原则**：不要在各命令里各写一套清单解析）。

### S9 `checkver` 深度对齐（先调研再定范围）

- **范围**：报告 §7.1（52 条）。
- **建议再拆**：①URL 解析与快捷分支（github/sourceforge/变量替换）；②提取链（jsonpath/xpath/regex/replace/reverse）；③autoupdate 变量表与属性替换范围；④hash 提取链（`lib/autoupdate.ps1` 全套兜底正则/模板/base64/fosshub/RDF）；⑤输出与 `-ThrowError`。
- **决策门**：先给"线上真实 bucket 会踩到的"优先级清单，再定做哪些（52 条不必全做）。
- **方法建议**：用真实 manifest 做对照夹具（几类典型 + 每类真实样例），逐例比对，不允许"以后再对齐"。

### S10 `checkhashes` 深度对齐

- **范围**：报告 §7.2（22 条）。
- **决策门**：缓存策略（hok 默认复用 vs 官方默认重下、`-UseCache`/`-k` 语义）、空 hash 处理、`all_urls()` 合并是否都对齐。
- **风险**：写回 manifest 属破坏性动作，需 `hok-write-json-trailing-newline` 约定的 CRLF/末尾换行校验。

### S11 其余 hok 独有命令

- **范围**：`checkurls`（§7.3，14 条）、`missing-checkver`（§7.4，8 条，`-s` 语义相反须先定）、`virustotal`（§7.5，10 条，含退出位掩码，与 S1 联动）、`formatjson`（§7.7，含"子命令名被 kebab 化为 `format-json`"这一实际 bug）、`ci-auto-pr`（§7.7，9 条）。
- **建议**：每条命令独立会话；`formatjson` 的命令名问题可先单独核实（可能是最容易被用户踩到的）。

### S12 插队：`ignore-failures` 默认显示疑似 bug

- **范围**：报告 §8 末条（`config.rs:404` 显示 `true` vs `:1122` 运行时 `false`）。
- **做法**：实测 `hok config list --all` 与运行时行为 → 若是 bug，最小改动 + 测试 → 单独提交。

### S13（可选）安装/卸载批量行为

- **范围**：报告 §4.4、§4.5。
- **依赖**：S1 的退出码口径。
- **决策门**：运行中进程（hok 中止整批 vs 官方跳过继续）、未安装应用（exit 1 vs 0）、held 包跳过、拒卸被依赖包——四条是否全部对齐，还是保留 hok 的安全行为（**保留也可以，但必须写进文档并登记为有意偏离**）。

### S14（可选）`bucket` / `shim` 参数与退出码

- **范围**：报告 §6.2、§6.5。
- **决策门**：`shim -g` 位置是否放宽、`shim list *` 是否容错、bucket 是否补重复仓库检测与 git URL 校验、`shim` 退出码是否对齐 3/2。
- **说明**：若 S7 已做，可与之合并成一次"参数/退出码/文案"批量会话。

---

## 4. 不做清单（有意偏离，除非你明确要求）

- `info` 的 4 条（bin 单元素、shortcuts 裸串、Download size 架构、`-v` 复用全局 verbosity）；
- `checkver` Digest 头解码失败回落 `.meta4`；
- `formatjson` 的 JSON5 宽容度与不做 `ConvertToPrettyJson` 反转义；
- `checkhashes` 对象形式 hash 回写刷新；
- `parse_json` 严格报错（而非 warn→`$null`）；
- `reset` 多跑 `post_install`；
- hok 单二进制设计带来的：无 scoop 自更新、无 `-u/--no-update-scoop`、`scoop_repo`/`scoop_branch`/`shim` 配置键 N/A；
- `upgrade` / `reinstall` / `completions` 作为 hok 独有命令保留（但 `upgrade -f` 语义冲突建议在 S7/S14 顺手改）；
- **下载路径不发 `Referer`**（官方 `lib/download.ps1:94-96` 会发 `strip_filename $url`）：腾讯 `updatecdn.meeting.qq.com` 带它 403、不带 200，官方同款缺陷（ScoopInstaller/Scoop#6009、#6212），需要 Referer 的站点走 `PRIVATE_HOSTS`。落地 `cf260d9`，详见 `.agents/skills/scoop-gap-audit.md` 2026-09-13「下载默认不发 Referer」节。

---

## 5. 通用要求（每条会话都适用）

1. **动手前**：全局 `investigate-first` — 实测复现 + 候选方案（含"不改"）+ 代价 + 影响面，等确认。
2. **改动中**：`no-patchwork-fixes` — 修语义错位本身，不加开关掩盖；共享逻辑走单入口（清单解析 = `internal::json` / `Get-Manifest` 等价入口）。
3. **收尾**：`rust-deterministic-gate` + `hok-rust-gate-practice`（独立 rustfmt `--edition 2024`、勿传含 `mod` 声明的文件、`git diff` 圈定文件再 fmt、仓库有既有 fmt diff 时禁整包 `cargo fmt`）。
4. **提交**：`git-commit-practice`（一主题一 commit；hunk 拆分用 `git apply --cached`）、`git-signed-commits`。
5. **提交前**：全局 `proactive-review`（对照官方源码、边界/失败路径/幂等、架构合规、测试覆盖、提交完整性）。
6. **文档**：更新本计划 §6、报告对应行、`.agents/skills/scoop-gap-audit.md`；新增报告按 `hok-report-archive` 归类（活文档留 `.agents/docs/` / 失效报告入 `.agents/docs/reports/`），报告产物不入库。
7. **并行 worktree（2026-09-13 起强制）**：全局 `worktree-parallel` —— 一会话一 worktree（`.worktrees/<会话>-<slug>`，base 取主线**当前** tip）一主题；**主线 `main`** 只接受线性 `--ff-only`，禁 merge commit（后落地者 rebase + 解冲突并保留双方语义；一条命令用 `~/.dsh/skills/worktree-parallel/scripts/worktree-merge.ps1`，自测同目录 `test-worktree-merge.ps1`（须在仓库根运行））；各 worktree 用自己的 `target/`，验证只信自己刚构建并复制的二进制；**§6/报告/技能等汇总型热点文档只在主树改、写入前重读、行级替换**；禁 `git add -A`、禁 `git clean -fd`。

---

## 6. 进度追踪

| 会话 | 状态 | 决策结论 | 提交 | 备注 |
|---|---|---|---|---|
| S1 退出码/失败策略 | **已完成** | **A 全面对齐官方逐命令退出码**：clap 用法错/缺参 exit 2→1、裸 `hok`→帮助 exit 0；search 无匹配=1、list 全空=1、bucket 空=2 / 已存在=2、shim `add` 目标缺失·`rm`·`info`=3（跨作用域 2）、virustotal manifest 缺失=位 8、download 部分失败=0 | `4a1fd5a`（clap 层）/ `e7d1e25`（逐命令）/ `cfd44ed`（download），worktree `.worktrees/s1-s13`，分支 `fix/exit-code-and-batch-policy` → **已 rebase 到 `3c90095` 后线性 ff 进 `refactor/edition-2024`（tip `cfd44ed`，无 merge commit；与 S2 在 `src/cmd/mod.rs`/`src/lib.rs` 的冲突按预告解决）** | 语义相关的退出码仍归各自会话（报告 §10：`cleanup` 无参、`reset` 未安装、裸 `cache`、`shim`/`alias` 默认子命令、`hok -v`）；新增 `tests/exit_codes.rs` 进程级回归。⚠️ **与 S2（`3c90095`）在 `src/cmd/mod.rs` 同处冲突**：合并时保留 S2 的 `ErrorKind::InvalidSubcommand` → alias 分发，未命中再回落 `exit_for_parse_error(err, &raw)`（**勿保留 `err.exit()`**，否则退出码退回 clap 2）；`normalize_subcommand_case` 形参已改为 `&[OsString]` |
| S2 alias | **已完成** | A：官方式脚本别名（`shims\scoop-<name>.ps1` + config 存脚本名）；附带项全选（重名/不存在报错 exit 1、拒绝内建重名、去 60 字符截断、裸 `alias` 保留 list 登记有意偏离） | `3c90095`（worktree `.worktrees/s2-alias`，分支 `fix/alias-availability`） | 详见 `.agents/docs/reports/s2-alias-investigation-2026-09.md` §7；预估的 shim 子系统耦合经核实不存在（官方 `scoop shim list` 本就 glob `*.ps1`）；原计划"两个 commit"合并为一个（CLI 必须与 libscoop 存储改造同时切换才可编译） |
| S3 export/import | **已完成** | **单一官方 schema + `--hok` 扩展**：`export` 输出官方数组形状（含全局 app、isolated、`Updated`、`Info`，时间戳官方 round-trip 形状；删 `-a`）；`--hok` 追加顶层 `hok`（自身配置 + 别名命令）与逐 app `Hok`（Hold/Global/Architecture/Failed），官方会忽略故仍可被 `scoop import`。`import` 读官方数组 + hok 旧 map，只导入兼容子集并逐条报告原因（bucket 非 git/已存在、config 不支持或机器相关键、`Install failed` app、`<auto-generated>` 缺版本、bucket 不可用），保持 `AssumeYes`+`IgnoreFailure` 与 exit 0，按实际安装结果汇总 | `a61a471`（光标转义仅在终端）/ `110e160`（export 官方形状）/ `2dd84b2`（import 兼容子集 + 报告）/ `14b9c29`（别名随扩展段往返），worktree `.worktrees/s3-export-import`，分支 `fix/export-import-interop` → **已 rebase 到 `140d04a` 后线性 ff 进 `refactor/edition-2024`（无 merge commit；与 S2 在 `locales/*.yml` 无冲突）** | 详见 `.agents/docs/reports/s3-export-import-investigation-2026-09.md` §9–§11；`-c/--config`（官方键名导出）随 S4。实测：`hok export` 72 app ↔ 官方脚本复演 72 app、`Info` 逐条一致（broken app hok 记 `Version: null`，官方记 junction 目标名，导入同样不可用）。⚠️ 落地时按 skill §5 手工执行等价检查（`merge-base --is-ancestor` + `git diff --quiet <已验证 tip> HEAD` = 0），未用 `.agents/scripts/worktree-merge.ps1`（脚本当时尚未出现在预期路径，收尾时已确认位于被忽略的 `.agents/scripts/`，下次直接用它；2026-09-13 起该脚本已迁至全局技能包 `~/.dsh/skills/worktree-parallel/scripts/`） |
| S4 config | **已完成** | **严格键名 + 官方式 CLI 形态**（决策：①键名只认官方拼写——下划线，`aria2-*` 连字符，砍掉 hok 别名；②未知键分流：官方独有键 → 新 `Error::ConfigKeyUnsupported`，非设置项 → `ConfigKeyInvalid`；③子命令只留 `edit`（hok 扩展）与 `rm`（官方名），`set`/`unset`/`list` 删除，写入走 `hok config <key> <value>`；④输出三档：裸 `config` = 文件里保存的键（`key = value`）、`-j` = 同一份 JSON、`-a` = 全键表格；⑤`-h` 不对标，设置手册继续走 `--help`）。顺带 **S12**：`ignore-failures` 默认值显示/帮助/README `true`→`false`（+ 守卫测试）；**S3 遗留**：`export -c/--config` 官方键名 config 段，并修正 S3 键名表的前提错误（官方是下划线规范名，原 kebab 表导致真实 `scoop export -c` 的配置段基本读不进）。**暂缓**：`aria2-min-split-size` 20M→5M、`proxy` 三态（`none` 字面值/直连 + `default`/`currentuser` 不再打挂下载） | `38ecfa9`（S12）/ `f4300aa`（单键读取+裸 `config`+`rm`）/ `4cefb81`（键名表修正）/ `b7c1680`（报错分流）/ `7987205`（export `-c`）/ `98e28a1`（严格拼写）/ `cf1baec`（CLI 形态），worktree `.worktrees/s4-config`，分支 `fix/config-key-and-subcommand` → **已 rebase 到 `cc2c072` 后线性 ff 进 `refactor/edition-2024`（tip `cf1baec`，无 merge commit；与 S5/S6 在 `locales/*`、README 无冲突）** | 详见 `.agents/docs/reports/s4-config-investigation-2026-09.md`；⚠️ **破坏性变更**：`hok config set`/`unset`/`list`/`list --all` 与下划线别名（`aria2_split` 等）不再可用，改用 `hok config <key> <value>` / `rm` / `-a`。实测：隔离环境逐条验证（裸/`-j`/`-a`/读/写/`rm`/冲突/别名报错）+ `export -c`↔`import` 往返；`cargo test --workspace` 仅 2 条 ACL 环境性失败（clean HEAD 同样失败），clippy 新增 0 |
| S5 checkup | **已完成** | **B&C 两者并存**：`checkup` 对齐官方环境检查（管理员+Defender 性能×2、main bucket、长路径、开发者模式、7zip/innounp/dark、双根 NTFS），**恒 exit 0**（对齐官方）；hok 原已装包体检迁为新命令 `health`（顺带修掉 `checkup` 的不可达分支）；Defender 用**纯 Rust**（WinDefend 服务状态 + Defender 注册表读实时/排除项），读不到即降级不误报；`use_external_7zip` hok 未建模 ⇒ 7zip 恒按需检查 | `5ae994d`（worktree `.worktrees/s5-checkup`，分支 `fix/checkup-semantics`） | 详见 `.agents/docs/reports/s5-checkup-investigation-2026-09.md`；新增 `package::diagnostic`（纯 `evaluate(&EnvFacts)` + 10 条单测）与 `internal::os` 注册表/服务/卷探针；`package::checkup`→`package::health`；隔离 `USERPROFILE` 实测：`checkup` 6 问题 exit 0、`health` 命中原本不可达的"无 apps 目录"分支 exit 0；`cargo test --workspace` 仅 2 条沙箱 ACL 环境性失败（clean HEAD 同样失败），clippy 新增 0、rustfmt 干净 |
| S6 cleanup/cache | **已完成** | 三项全选：①数据安全——`cleanup -k` 逐 app 保留**当前版本**缓存 + 末尾清 `*.download`（不再清空整缓存）；`cache rm`/`list` 匹配改为**按 app 名锚定**（大小写不敏感全等，`rm zip` 不再命中 `7zip`）且支持多 app（`cache rm zip 7zip`）；按 app 删除同时清 aria2 sidecar `<app>.txt`；②`-g` 走共享 `ensure_global` 管理员门（exit 1，文案逐字对齐）、`-a -g` 改为本地 + 全局两趟；③**破坏性对齐**：裸 `cleanup` 报 `<app> missing` exit 1、裸 `cache` / `cache <app>` 默认列缓存（exit 0）；命名 app 走完整 `Confirm-InstallationStatus`（跨作用域 `--global` 双向提示 + `isn't installed correctly`，均 exit 0）。附带把 `cleanup` 目标列表改为**始终显式**（空列表 = 无事可做，不再等于"全部"，杜绝 `cleanup <未安装>` 清全盘） | `1e6b929`（cache 选择锚定）/ `33ba858`（-k 保留当前版本）/ `1c1311b`（-g 管理员 + `-a -g`）/ `cc2c072`（无参与未安装语义），worktree `.worktrees/s6-cleanup-cache`，分支 `fix/cleanup-cache-safety` → **已 rebase 到 `5ae994d` 后线性 ff 进 `refactor/edition-2024`（tip `cc2c072`，无 merge commit；与 S5 无冲突）** | 详见 `.agents/docs/reports/s6-cleanup-cache-investigation-2026-09.md` §7；隔离 `SCOOP*` 环境实测 10 条。有意/微偏离：`cache` 用字面全等而非官方 regex 元字符；缺参报错用提示文案替代官方 usage 块；broken app（`current` junction 缺失）报"未正确安装"后**跳过**（官方会删光其版本目录，hok 保留更安全行为）；输出样式（逐文件 `Removing ...` / `Deleted: N files`）留给 S7。⚠️ 复现过的坑：`package::cleanup::cleanup` 原先"空名单 = 全量扫描"，命名路径过滤后传空名单会**清空所有已装 app**——已改为显式名单 + `installed_app_names` 枚举 |
| S7 低风险批量 | **已完成（本批范围）** | 见下方「S7 + S14 落地记录（2026-09-13）」 | `a5c4f73`（标志/命名）/ `842bb5a`（status+list 文案）/ `64646bf`（search·download·reset）/ `cbb7d1d`（update）/ `56d9c1d`（bucket 表）/ `7919c03`（status 标题）/ `9cf35d2`（style）/ `ef1ff6c`（install + `PrunedInstall`）/ `9c6e633`（cache）/ `46a9e82`（cleanup）/ `310b45c`（list Updated）/ `aed0734`（shim 表），worktree `.worktrees/s7s14-low-risk`，分支 `fix/s7s14-low-risk` → **已线性 ff 进 `refactor/edition-2024`（tip `aed0734`，12 提交，无 merge commit；`worktree-merge.ps1` 漂移校验：提交数/主题/改动文件集守恒 + 落地树与已验证 ref 逐字节相同）** | 详见 `.agents/docs/active/audit/s7-s14-investigation-2026-09.md`；含 **4 项破坏性变更**；S14 的参数/退出码类已划出到另开会话 |
| S8 路径解析类 | **已完成** | **A1 完整对齐 + B1 单入口 + D1 + E1，C1/F2 有意保留**（决策门全答）：①`which` 输出 shim 指向的真实目标、经 PATH 解析（`.ps1`→`%PATHEXT%`→裸名；顺序实测于 PS 5.1 + pwsh 7），同时分类 user+global shims、只输出一个；**简化项**（不建模 PS alias/函数、不查当前目录）与**不套 `friendly_path`** 登记有意偏离；**全局 `scoop-*.ps1` 别名反向偏离**：实测官方 `Get-CommandPath scoop-ghello`（global）返回 `$null`（官方 `which` 报 not found），hok 两作用域都返回脚本路径。②`cat`/`home`/`info` 全部改走新公开单入口 `manifest_source::get_manifest`（完整复刻 `lib/manifest.ps1:34-112`：URL/UNC → 已安装记录（`install.json` 的 bucket → bucket 清单 + `<bucket>\deprecated` 回退；无 bucket → `url` 远程/本地 → installed manifest）→ 未安装（`parse_app` 拆 `bucket/name@version` → 显式 bucket 只读该 bucket → 否则逐 bucket 扫描 → 本地文件）；UNC 按本地文件读（官方 `WebClient` 必失败，属超集）。③**多 bucket 命中保留 hok 交互选择**（用户决定，不删交互代码）——**暂缓/待定**，非"有意偏离已定"；已知限制：**非 tty（脚本/CI）下交互失败 exit 1，等于该命令在有重名包时不可用**。④`prefix` 补全局回退 + 统一单条文案 `Could not find app path for '<app>'.`。⑤`home` 无 `homepage` 改报官方 abort（实测原来是**静默 exit 0 无输出**，比审计描述更糟）。⑥`cat` 美化（hok 2 空格/LF/不折叠单元素数组）与 bat 缺失回退登记有意偏离 | `c5b21d2`（单入口）/ `41ddaeb`（which）/ `4b25087`（prefix·cat·home），worktree `.worktrees/s8-path-commands`，分支 `fix/s8-path-commands` → **已线性 ff 进 `refactor/edition-2024`（tip `4b250876`，3 提交，无 merge commit；`worktree-merge.ps1` 漂移校验：提交数/主题/改动文件集守恒 + 落地树与已验证 ref `4b25087` 逐字节相同；与 S7 在 `locales/{en,zh}.yml` 冲突按"双方语义都保留"解决——取 S7 的 `update_latest`/`update_all_latest` + 本条的 `prefix_not_found`，删已无引用的 `outdated`；并适配 S7 新增的 `shim_info` → `resolve_in_path` 返回类型由 `Option<String>` 改 `Option<PathBuf>`）** | ⚠️ **范围外副作用（方向正确，官方确认成立）**：`download`/`install` 的 URL/本地/`@version` 解析随单入口一并改用 `Get-Manifest`（官方 `install.ps1:9`、`scoop-download.ps1:62` 同样如此），即会读安装记录——回退它反而会给 download 单开第二条解析路径。⚠️ `get_manifest` 的多 bucket 候选按 **bucket 名排序**（hok `bucket_added` 并发无序），非官方目录枚举顺序。⚠️ 删除已无引用的 locale 键 `failed_scan_bucket`。实测：隔离 `SCOOP`/`SCOOP_GLOBAL`/`USERPROFILE`/`PATH` 环境 22 条用例 + 3 条 download 解析；libscoop 单测 11+9 条（`package/shim.rs` 新增 `command_path`/`classify_command_path`/`shim_target`）；`info` 回归 12 用例对 `aed0734` 二进制**逐字节一致**（193 行 vs 193 行；真实 root 只做元数据副本，**勿直连 `$env:SCOOP`**——hok 打开 `<root>\scoop.db` 会建表并切 WAL）；`cargo test --workspace` 651 passed，仅 2 条 ACL 沙箱环境性失败（clean base 同样失败）；clippy `-D warnings`（`--exclude innospect`）干净、文件级 rustfmt 干净。调研细节与脚本仍在 worktree 的 `target/`（`s8-verify.ps1`/`s8-info-regression.ps1`/`s8-info/before|after.txt` 等），待归档为 `.agents/docs/reports/s8-path-resolve-investigation-2026-09.md` |
| S9 checkver | **已完成** | 按推荐执行 **S9-2 → S9-3 → S9-1**（逐块确认、逐块实施）：①**S9-2** 变量表（`Substitution`，**最长键优先** + `$match<Name>` 大小写不敏感）——补齐 `$url/$baseurl/$basename/$urlNoExt/$basenameNoExt` 并按官方**逐 URL 分层**应用到 hash 配置，属性替换改为**驱动自 raw JSON 的 autoupdate 键**（覆盖 changelog/extract_to 等），hash 只在 manifest 已有 hash（顶层或对应 arch）时写入，hash 回退下载改落 Scoop 缓存（`Invoke-CachedDownload` 语义），删掉恒等 stub（整个 `checkver_url.rs` 移除）；②**S9-3** hash 提取链（默认兜底正则 `^\s*([a-fA-F0-9]+)\s*$` + 文件名旁 + metalink、`$sha256/$basename/$base64` 模板、`find` 改**正则取第 1 组**、"找不到"**回落下载 + 本地 SHA-256** 而非整条失败）；③**S9-1** github/sourceforge 快捷方式**显式建模**（`Checkver` 增 4 个 `#[serde(skip)]` 标记）、API 改写**仅在配置 token 时**（逐字对齐官方）、`checkver.script` 复用既有子进程入口 + `$json/$app/$expected_ver` preamble、`--throw` 落地并与 dflag 的 `-d` 写盘护栏共存。**④ 与已落地 D4 去重**：请求头一律走共享 `ScoopRequest`/`HeaderProfile`（删 S9 内联 `RequestOptions` 与自读 `private_hosts`/`gh_token`/会话 UA，`ResolvedSource` 删 `user_agent`/`token` 字段，`download_bytes` 改走 `HeaderProfile::Autoupdate`），只保留"快捷方式形态 → useGithubAPI"这一条 URL 层看不到的判定 | `430f602`（S9-2a）/ `204cf79`（S9-2b）/ `982934e`（S9-3）/ `8ad7d08`（S9-1），worktree `.worktrees/s9-checkver`，分支 `fix/s9-checkver`（base `be0092d`）→ **已线性 ff 进 `refactor/edition-2024`（tip `8ad7d080`，4 提交，无 merge commit；重放用 `cherry-pick -n` + `commit -C`，与 dflag 在 `src/cmd/checkver.rs` 的 import 行冲突按"双方语义都保留"解决；落地时先修正了该分支引用停在旧位置 + worktree detached 的状态）** | **实测量化（21797 个真实 manifest / 4 bucket）**：hash 变量从不替换 **1160/3849** manifest（条目级 4102 条 hash url、1770 条 hash regex）→ 全部可解；hash 链无解 **1350/1387** 条目（1298/3846 manifest）→ 全部可解；github+自定义 re 被喂 API JSON **122/620** → 恢复；联网逐例与已安装 Scoop 0.5.3 **请求 URL 逐字一致**（`ag`/`7zip`）；约 **999 个** `/releases` 双写清单恢复可用。门禁：libscoop **744 passed**（仅 2 条既有 ACL 失败）、`cargo test -p hok` 全绿、clippy 0 warning、rustfmt 干净、新增 40+ 离线夹具（含端到端 FakeNetwork）。**⚠️ 两条有意偏离（登记）**：**`Bearer`**（用户拍板：GitHub 标准写法；官方 `bin/checkver.ps1:223` 用 `token`，两者皆收）；**`checkver.github` 已含 `/releases`（或 `/releases/latest`）时不再追加**（官方实测产出 `.../releases/releases/latest` 必 404，hok 做**超集修复**——官方在该形态本就不工作，故不破坏任何"官方能工作"的场景）。**⚠️ 过程教训**：本沙箱里提交信息一律 `-F <文件>`（PowerShell 会展开反引号与 `$`，该会话曾损坏 3 条信息并用 `git fsck --unreachable` 找回重建）；有未提交改动时**绝不** `reset --hard` |
| S10 checkhashes | **已完成** | 决策：①缓存 **A1**——**默认行为不动**（实测 hok 默认已是"每次重下 + 跑完删临时目录"，与官方默认一致；原报告"hok 默认复用/跳过下载"**结论有误**，见更正条），只把 `-k` 的缓存键修为含版本 `<cache>/<version>/…`（≡ 官方 `-UseCache` 的 `<app>#<version>#…`），`-f` 覆盖 `-k`，`--help`/README 写明 `-k` ≡ `-UseCache`（可能拿到旧文件）；②空 hash **B1**——`""` 按 sha256 期望比较（必然 mismatch）→ `-u` 写回真实 hash，数组逐条独立、去掉 `break`；`TODO`/未知算法/`{url,hash}` 对象**不再静默 skip**，改显式报解析失败（**不放宽 `HashString` 解析层**，避免波及 install/check_hash）；③url 选择 **C1**——顶层 `url` 优先，无顶层才 64bit→32bit→arm64，**新增** `Manifest::checkhashes_accessors()`，**不改 `all_urls()`**（checkurls 仍用）；④小项：`-App` 大小写不敏感（共享 `matches_any_glob`，**与 S11 同源**）、`nightly` 大小写不敏感、无 URL 打印官方文案 `Manifest does not contain URL property.`、mismatch 明细补 `First bytes` + 完整 Expected/Actual、数量不匹配不再误记 failed | `e859192`（文档/长帮助/`.s10lab/` 忽略）/ `6e142ef`（大小写不敏感过滤）/ `4e990a0`（checkhashes 主体 + 新访问器 + CLI + locales）/ `ad5d253`（清死计数器），worktree `.worktrees/s10-checkhashes`，分支 `fix/s10-checkhashes`（base `cf1baec`）→ **已 rebase 到 `4b250876`（重放无冲突，提交数/主题/改动文件集守恒），在重放后的树上重跑门禁后线性 ff** | 实测（官方脚本本地直跑对照 15 夹具）：①缓存请求计数 默认 1→1（官方 1→1）/ `-k` 1→0（官方 `-UseCache` 1→0），缓存键 官方 `c1#1.0#12d19fb.zip` ↔ hok `<cache>/1.0/`；②`-u` 写回 CRLF+末尾 CRLF，`hash:""` 夹具写回与官方**逐字节相同**（sha256 `84911644…`，154 B，`IDENTICAL: true`）；③`m7-both`（顶层正确 + 64bit 错）现 ✓（官方亦 OK）；④`M1-OK`/`M6*` 命中、`"Nightly"` 跳过；⑤`package::checkhashes` 15 + `internal::string` 17 单测通过。**有意偏离（登记）**：`handle_special_urls` 不做（需联网交互）；`-d` 默认 `.` 与多 glob 保留；末尾汇总行 / 缓存放 `%TEMP%\hok-checkhashes`（不污染用户真实缓存）/ `-c/--cache` 扩展 / 下载失败继续 保留；`-u` 只在有 mismatch 时写回（官方无条件重写**全部** manifest，hok 更安全）、写回只动被改文件并保留原行尾。**⚠️ 更正**：审计报告 §7.2「hok 默认跳过下载」「hok 默认复用」两行经实测**不成立**。**划出**：D4 `download_file` 缺 UA/Referer/cookies/PRIVATE_HOSTS（跨命令缺陷）→ 已另开会话 `fix/d4-download-headers` |
| S11 其余独有命令 | **本批（formatjson + missing-checkver）已完成；其余未开始** | **① formatjson 命名 = A**：`#[clap(name = "formatjson", alias = "format-json")]`（官方名为主 + 兼容别名）；同时修掉 `hok-i18n-derive` 按 kebab 变体名打补丁、而 clap 按最终注册名匹配导致的**启动 abort 共享 bug**（`ci-auto-pr` 是同类潜在坑）。**② missing-checkver = 完全对齐官方**：`-s/--skip-supported`（语义反转，**破坏性**）、默认输出官方三行图例 + 逐行 `[C][A] name` 真值表并**保留 hok 汇总行**（新增 `unparsable` 单列一档）、不可解析清单出行 `[ ][ ] name` + 官方 `Error parsing JSON at '<path>'.`、补 app 过滤（**位置参数 `[APP]...`**，非 `-a/--app`——官方 `bin/missing-checkver.ps1:13` 为 positional，且 hok 的 checkurls/checkhashes/checkver/formatjson/virustotal 全部是 positional）。**③ 共享管线**：`internal::string::matches_any_glob` 改为**大小写不敏感 + 忽略 `.json` 后缀**（按官方 `Get-ChildItem -Filter "$App.json"`），`checkhashes`/`checkver`/`checkurls`/`formatjson`/`missing_checkver` 共用 | `5b0b4cc`（共享管线）/ `21c7d65`（formatjson 命名）/ `cd91d8a`（missing-checkver 对齐），worktree `.worktrees/s11-formatjson`，分支 `fix/s11-formatjson` → **已 rebase/重放至 `e97744a`（重放后 tip `2a18d5b`）；与 S10 在 `internal/string.rs` 冲突按"保留超集"解决（取 S11 的大小写不敏感 + 忽略 `.json`，并保留 S10 的 `filters_are_case_insensitive` 回归测试），与 S7 在 `locales/*.yml` 冲突按"双方语义都保留"解决；重放后在树上重跑门禁（666 passed / 2 已知 ACL 失败）后线性 ff 进 `refactor/edition-2024`（tip `2a18d5b`，3 提交，无 merge commit）** | ⚠️ **破坏性变更**：`missing-checkver -s` 语义反转（原"只列有 checkver 或 autoupdate 的" → 现"跳过两者都有的"）；默认输出由"只列缺字段 + 汇总"改为官方全量真值表；`matches_any_glob` 变宽容后 `checkver`/`checkhashes`/`checkurls` 的 app 过滤同样放宽（仅放宽不收紧）。**⚠️ 唯一行差异**：官方真值表会多列一行 `[ ][ ] package`（hok 有意跳过 `package.json`）。**遗留**：`checkurls`（§7.3）、`virustotal`（§7.5，含退出位掩码）、`ci-auto-pr`（§7.7）仍未做 |
| S12 插队小项 | **已完成（并入 S4）** | `ignore-failures` 的默认列 / locale 长帮助 / 两份 README 由 `true` 修正为 `false`（与运行时一致），并新增守卫测试（新配置下每个 bool 行的默认列必须等于 getter） | `38ecfa9`（随 S4 分支线性 ff 进 `refactor/edition-2024`） | 原登记为"疑似 bug"，实测确认：`53e35f3` 只改了 getter 默认值，其余三处漏改。本行 2026-09-13 由父会话复核后回填（S7 收尾时遗漏） |
| S13（可选）批量行为 | **已完成（本轮范围）** | 运行中进程=**跳过该包继续**（不再中止整批）+ `ignore_running_processes` 按官方语义（默认跳过；该键为 true 时**警告后继续**）；**⚠️ 2026-09-13 晚更正**：本条最初写成「只定文案、仍跳过」，系误读官方 ignore 分支 `return $false`（`Scoop/lib/install.ps1:539-542`，`$false` = 未在运行 = 调用者继续）；同日已回正（另含 WARN 取代 ERROR、文案回 `Running process detected, skip …`、进程明细仅 `-vv`），见 `.agents/skills/scoop-gap-audit.md`「运行中进程输出 + ignore_running_processes 语义回正」；未安装=**报错后继续其余包、exit 0**；`scoop` 自身不加特例（保留 hok 单二进制语义，有意偏离）；**held 跳过 / 被依赖拒绝本轮不改**（登记为有意偏离） | `a4ea0b7`（与 S1 同分支 `.worktrees/s1-s13`，随其线性 ff 进 `refactor/edition-2024`） | `Error::AppRunning` 随语义修正删除；libscoop 单测 **`check_not_running_skips_by_default_and_continues_when_ignored`**（2026-09-13 晚随语义回正重写，含有序事件断言：WARN 在前、进程明细在后）+ e2e 实测（`a4ea0b7` 当时：默认 config 报 ERROR / 开 `ignore_running_processes` 报 WARN，**两种都跳过**且 exit 0；**回正后**：默认跳过、开键**继续**，两者都只 WARN，进程名+pid 仅 `-vv` 输出）；broken 安装（无可解析版本）仍走官方"跳过检查 + 删目录失败即报错"路径 |
| S14 bucket/shim 参数与校验 | **已完成** | **四项均按推荐（2026-09-13）**：①bucket 全对齐——`internal::git::normalize_git_uri`（官方 `Convert-RepositoryUri` 正则 **1:1 移植**，校验与重复比较共用、**大小写不敏感**）+ 重复仓库 `WARN Bucket <b> already exists for <repo>` / **exit 2** + 添加流程 `Checking repo... `（无缩进）`OK` + 成功文案 + `ls-remote` 预检失败官方多行 / **exit 1**；②URL 校验两行 ERROR **逐字**（`<repo> is not a valid Git URL!` + `Please see https://git-scm.com/docs/git-clone#_git_urls for valid ones.`）；③shim 三项全做（`-g` 加 `global = true` 任意位置可写 / `list -g` 只列 global / `list *` 剔除字面 `*`），并**接受破坏性变更**；④作用域落 lib：`list_shim_info(session, global_only)`，`-g` 时连 local 目录都不扫。顺带对齐 `shim add` 缺 `<command_path>` 的官方文案。**不做**：clone 失败 `Remove-Item` 清理（实测 libgit2 不残留）、无 git 时 `Git is required`（内嵌 libgit2） | `19a8e6e`（URL 校验 + 重复检测）/ `581df3f`（添加流程输出）/ `127c903`（`ls-remote` 预检）/ `dca2b51`（shim `-g`/作用域/通配），worktree `.worktrees/s14-bucket-shim`，分支 `fix/s14-bucket-shim`（base `eb67941`）→ **已线性 ff 进 `refactor/edition-2024`（tip `dca2b51e`，4 提交，无 merge commit；漂移校验：提交数/主题/改动文件集守恒 + 落地树与受测 ref 逐字节相同）** | 隔离 `SCOOP`/`SCOOP_GLOBAL` 进程级实测全绿（bucket 成功 / 重复=2 / 非法 URL=1 两行 / 死仓库=1 多行 / 未知名=1；中英双语 `Checking repo... OK`；shim `list -g`≡`-g list` 只列 global、`list *` 全列、`info -g` 跨作用域=2、`rm ghost -g`=3、`add` 五种 `-g` 位置与 `-- -g` 转发均符合官方语义）；官方 16 例 URL 求值做成单测 `normalize_git_uri_matches_upstream`（PS 5.1 实测值逐条断言）。⚠️ **实现期两个新坑**：①**git2 0.19 `RemoteConnection::list()` 对零 refs 仓库会 abort 进程**（`from_raw_parts(null,0)` 触发 Rust 非空指针前置条件，debug `0xC0000409`、release UB）⇒ `ls_remote` **只做 `connect_auth` 握手、不调 `list()`**，空 bare repo 有回归单测；②`Add.parts` 的 `num_args = 2..` 必须改 `1..`（`-g` 夹在值中间会让 pending 组少一个值而报 clap 通用错）⇒ 缺 `<name> <target>` 改由 handler 报官方文案。**⚠️ 破坏性变更**：`hok shim add x cmd -g` 由「`-g` 当 shim 参数、建 local」→「建 global」，`-` 开头的 shim 参数必须走 `--`；`shim add <只有名字>` 文案由 clap `2 values required` → 官方 `<command_path> must be specified for subcommand 'add'`（仍 exit 1）；`bucket add` 现在拒绝官方正则不认的 URL（exit 1）与重复仓库（warn + exit 2）。**有意偏离**：clone 失败仍打 libgit2 原文（官方 `Failed to clone …` 未对齐，登记遗留）；`Checking repo... ` 与 ERROR 分属 stdout/stderr（官方都在 stdout，hok 保持 stderr 约定）；成功行带 hok `✓ ` 前缀；`bucket list -k`/`rm` 多名称、`shim` 无子命令默认 list / 无 `alter` / 逐 pattern 校验改合并正则保留 |
| **D4（S10 划出）下载请求头** | **已完成** | 用户 2026-09-13 拍板 **D4b 分两组**：新增唯一入口 `libscoop::network::ScoopRequest` + `internal::network::HeaderProfile`（`Download`/`CheckUrls`/`Checkver`/`Autoupdate` 四变体，每个方法带官方 file:line），一次修掉 **4 套互不一致的 UA 语义**（`download_file` 一个头都不发、`head_digest_sha256` 真·无 UA、`head_url` 忽略会话 UA、`github.rs` 固定 `hok`）；UA 单点 = `constant::DEFAULT_USER_AGENT` = `Scoop/1.0 (+http://scoop.sh/) Hok/0.2.0`（**不伪造 PowerShell 段**——已查证真实源只有 GitHub API「必须非空 UA」一条硬约束）；Referer = `strip_filename(url)`、**从不使用 bucket**，下载路径对 `sourceforge.net`/`portableapps.com` 抑制（照抄官方不对称）；**顺带修安全项**：checkver 的 GitHub token 原本会发往任意 URL → 现只在 `api.github.com`；`checkver.useragent`（解析却从不读取）现生效。**Cookie 经查证为 no-op**（官方 `checkhashes.ps1:122`/`autoupdate.ps1:319` 都传 `$null`）⇒ 原计划的"+cookies"**不成立** | `e326ecf`（单入口）/ `6db6bfb`（GitHub API 头），worktree `.worktrees/d4-download-headers`，分支 `fix/d4-download-headers`（base `2a18d5b`，收尾 rebase 到 `dca2b51`，与本次 9 文件零漂移）→ **已线性 ff 进 `refactor/edition-2024`（tip `6db6bfbd`，2 提交，无 merge commit；漂移校验：提交数/主题/改动文件集守恒 + 落地树与受测 ref 逐字节相同）** | 本地夹具 `127.0.0.1:18877` 逐字断言 **18 passed**（四条路径 UA/Referer 前后对照）；命令回归 `install`/`download`/`reinstall`/`update`/`checkhashes`/`checkurls` 全 exit 0；`cargo test --workspace` 678 passed / 2 failed（已知 ACL 环境性失败）。**⚠️ 限制**：本机**出站 HTTPS 不可用**（直连与代理 `127.0.0.1:7890` 均 TLS 失败，纯 HTTP 可用）⇒ **真实源冒烟未做**，改用本地夹具 + 回环自动化测试锁定。**遗留/有意偏离**：`handle_special_urls` 不做（划出）；PRIVATE_HOSTS 全入口注入（未按 ①② 拆开提交）；不伪造 PS 段。**⚠️ 另发现两条既有问题（基线二进制同样复现，建议另开会话）**：`hok checkver` 在本环境**挂起**（空目录也挂）；`USERPROFILE` 未设时 `internal/fs.rs:63` **panic** |
| **`-d` 写盘护栏（会话 `dflag-write-guard`，S11 其余前置项）** | **已完成** | **一条统一规则**：`-d/--dir` 缺省仍为 `.`，但"**本次调用会写盘**"时必须显式指定目标，否则 **exit 1**（**由写盘与否决定，不由命令身份决定**）。逐命令：`formatjson` 总是写 → 恒要求 `-d`；`checkhashes` = `update \|\| force`、`checkver` = `update \|\| force_update`（官方 `if ($ForceUpdate) { $Update = $true }`，`bin/checkhashes.ps1:52` / `bin/checkver.ps1:404`）→ 仅这两个开关下要求；`checkurls`/`missing-checkver` 从不写 → 保持默认 `.`；`ci-auto-pr`（写盘并提交）本会话不改但**规则同样适用**。判定方式 = `dir: Option<PathBuf>`（去掉 `default_value`）+ `src/cmd/shared_args.rs` 的 `write_dir`/`scan_dir` **单一入口**（字段名不变 ⇒ `cli_help.<cmd>.args.dir` i18n 键不受影响）；文案为 hok 双语短句并**说清原因**（官方只说"缺参数"）。附带：`formatjson` 写盘时**逐文件**打印 `Formatting <path>...` | `1037f3c`（护栏 + 3 条进程级测试 + locales + README）/ `1009956`（逐文件输出 + `FormatReport` 携带路径），worktree `.worktrees/dflag-write-guard`，分支 `fix/dflag-write-guard`（base `2a18d5b`）→ **已重放到 `6db6bfb`（沙箱无编辑器，走 `cherry-pick -n` + `commit -C`）后线性 ff 进 `refactor/edition-2024`（tip `be0092df`，2 提交，无 merge commit；与 S14 在 `tests/exit_codes.rs` 的"双方末尾追加"冲突按**两边都保留**解决）** | 实测：①**护栏前**隔离环境裸跑 `hok formatjson` 真的重写了诱饵树 —— `tsconfig.json`/`package-lock.json`/`target\debug\out.json` 三个文件 hash 变化，连隔离出的 `home\.config\hok\config.json` 也被改写（根因：`manifest_walker.rs:44-45` 递归收全部 `*.json` 仅排除 `package.json`，`formatjson.rs:57-70` 走通用 `serde_json::Value` 不判"像不像清单"）；②官方对照：`formatjson`/`checkhashes`/`checkurls`/`missing-checkver` 的 `-Dir` 均 `Mandatory=$true` → 非交互 exit 1，`checkver.ps1:85` throw；③写盘型缺 `-d` 五组（`formatjson`/`checkhashes -u`/`-f`/`checkver -u`/`-f`）全 exit 1 且**诱饵树逐字节未动**、给了 `-d` 正常、只读型缺 `-d` 仍按 `.` 扫；`--test exit_codes` 8/8、`formatjson_cli` 6/6、`i18n_keys` 3/3。**⚠️ 破坏性变更**：裸 `hok formatjson`、`hok checkhashes -u\|-f`、`hok checkver -u\|-f` 由"扫 cwd"变为 exit 1。**⚠️ 有意放宽（登记）**：官方对只读脚本（`checkurls`/`missing-checkver`）与不带开关的 `checkhashes`/`checkver` **也**强制 `-Dir`，本规则只对写盘强制。**⚠️ 有意偏离（登记）**：`formatjson` 逐文件输出（官方静默，hok 增值）。**⚠️ 与 S9 分支在 `src/cmd/checkver.rs` 会冲突**（S9 仍在飞）；既有问题 `hok checkver` 在"无清单声明 checkver（含空目录）"时**无限挂起** 已在 base `2a18d5b` 复现、与本批无关 |

| **update/upgrade 对齐（会话 `update-upgrade-align`）** | **已完成** | **c1–c5 按推荐 + 发现 A 换新文案 + 级联单开**：①**删 15 分钟 bucket 冷却**（`Config::update_cooldown_remaining[_at]` + 单测 + i18n 键 `bucket_cooldown` + 两份 README 的宣称 + 失败汇总里的 `--force` 提示），`-f/--force` 收敛为官方语义「即使没有新版本也强制更新」；②`-I` → `-i`（install / update / `SyncArgs`，描述改用官方 `Don't install dependencies automatically`）；③`--escape-hold` 四处 CLI + `SyncArgs` 字段 + `SyncOption::EscapeHold` + 4 个 i18n 键全删，hold 门按官方口径收口：**uninstall 不再跳过 held**（官方 uninstall 无 hold 检查），install 路径保留门但**不再静默**（新 `cmd.held_install_skip`，点名当前被锁版本并指向 `hok unhold`；`held_skip` 仍只服务升级路径），`resolve_cascade` 去掉 `escape_hold` 形参、held 依赖恒报 `PackageCascadeRemoveHold`；④升级引擎 `execute_upgrade` 从 `update.rs` 移入 `upgrade.rs`（`update` 反向引用，方向 update → upgrade）；⑤`upgrade` 补 `-i`/`-k`（**不补** `-U`/`-R`：`-U` 与引擎强制的 `OnlyUpgrade` 相抵，`-R` 是 hok install 侧开关） | `ab7ea6e`（引擎搬迁）/ `c2581d4`（删冷却）/ `c25e39c`（`-i`）/ `20575a5`（hold 口径 + 删 `-S`）/ `ba6f923`（upgrade 补 flag）/ `77475e3`（rustfmt），worktree `.worktrees/update-upgrade-align`，分支 `fix/update-upgrade-align` → **已线性 ff 进 `refactor/edition-2024`（tip `77475e3`，6 提交，无 merge commit；`worktree-merge.ps1 -VerifiedRef 77475e3`：无需 rebase、提交数/主题/改动文件集守恒、落地树与受测 ref 逐字节相同）** | 行为锚：官方显式 `scoop update` **无条件**同步自身+buckets（`libexec/scoop-update.ps1:397-398`），其 3h `LAST_UPDATE` 窗口只门控**隐式自更新**（`lib/core.ps1:1168-1178`）；hold 唯一闸门在 update 路径（`scoop-update.ps1:432-438`）；`-i` 见 `scoop-install.ps1:24` / `scoop-update.ps1:11`；`$independent`/`$use_cache` 确实传进升级路径（`scoop-update.ps1:378-385,393`）。**pacman 佐证**：`pacman(8)` 的 `-y` 只按"库是否最新"跳过、`-yy` 才是强制，从无客户端时间窗（冷却由 systemd timer 承担）；hok 的 15 分钟门是自加的 `1c53b2c`，原 hok 作者的 pacman 参照只在**输出风格**（`output-style pacman`）。**文案决策**：`held_skip` = 升级路径"保留原因"，install 路径语义不同（用户点名要装却被锁）⇒ 不复用，新增 `held_install_skip`。**登记 R5**：`-c/--cascade` 在 `5f2532e`（eventloop 统一）被误删、`SyncOption::Cascade`/`NoDependentCheck` 自此无构造者 ⇒ 级联卸载生产不可达，单开会话恢复 |

| **`cleanup` 帮助面 + `-l/--list`（会话 `cleanup-list-l`，2026-09-22）** | **已完成** | 用户点名四项：① 删 `cleanup` **命令私有**的 `-v/--verbose`，连同它门控的「已清理干净」报告与 `cmd.cleanup_already_clean`/`cli_help.cleanup.args.verbose` 两键**彻底删除**（`-v` 回落全局 verbosity，`cleanup -v`/`-v cleanup` 仍合法）；② 新增 **`-l/--list`** 列出现存版本（用户明确：不是替代 `-v`；输出「参考 `list -V` 简化」；裸 `-l` = 全部已装 app）；③ `-a/--all` 补「（相当于 '*'）」、`[APP]...` 删「（留空表示全部）」；④ `cache` 位置参数三处未翻译（顶层缺 `cli_help.cache.args.app`；子命令仍是字段改名前的 `list.query`/`remove.query`） | `cd67788`（cache i18n + `[ARG]...` 检查）/ `65b83ee`（cleanup `-l` + 删私有 verbose + 文案）/ `2eba651`（broken app 不再误标 `旧版本`，改「当前版本未知」）/ `daa1002`（判定上移 `cmd::version_note`，`list -V` 共用 + 不计入旧版本页脚），worktree `.worktrees/cleanup-list-l`，分支 `feat/cleanup-list-versions`（前两条 base = `e8c09f1`；第三条落地时主线已前进到 `f286051`，脚本自动 rebase 后在新树重跑门禁 + 实机复验再 ff）→ **已线性 ff 进 `refactor/edition-2024`（tip `daa10020`，4 提交，无 merge commit；三次 ff 漂移校验：提交数/主题/文件集守恒 + 落地树逐字节相同）** | **hok 独有扩展（登记）**：`-l/--list` 官方无此选项（`scoop-cleanup.ps1` getopt 只有 `agk`）；数据复用 `package::list::list_installed_versions`（与 `list --versions` 同入口），渲染为其减 源/更新时间 两列；只读 ⇒ 跳过 `-g` 管理员门、`-k` 与之 clap 冲突。门禁：hok 全绿（lib 50 / `cli_help_i18n` 3 / `exit_codes` 9 / `i18n_keys` 3）、libscoop 760 passed / 2 failed（沙箱 ACL 基线）、clippy + rustfmt 干净；隔离 SCOOP 根实测 6 条 + 实机 potplayer 双向复验。⚠️ 坑：首版裸 `-l` 沿用破坏性路径的 `all` 判定 ⇒ 静默无输出（已抽 `lists_every_app()` + 单测）；en.yml 新键值含 `": "` 必须加引号否则 build.rs panic；`旧版本` 对 broken 安装是**越权断言**（已改中性提示） |

| **`cleanup -k` 缓存面三修（会话 `s-cleanup-cache`，2026-09-25，用户实机反馈）** | **已完成** | 用户报「`hok cleanup * -k` 貌似没同步删缓存」。**先证伪**：按真机布局（74 app + 25 缓存文件，目录/junction/元数据/缓存名全镜像）跑 `cleanup * -k` ⇒ 旧版本目录与过期缓存都被正确删除，参数顺序 `-k *`/`* -k`、经 shim 调用、persist junction 三类用例全过 ⇒ 主路径无 bug。**仍修三处真缺口（用户拍板全修）**：① `REGEX_CACHE_FILE` 字符类 `[0-9a-zA-Z._-]+` → `^[^#]+#[^#]+#`（对齐官方 glob；含 `+`/`_` 的版本号与 CJK 包名的缓存文件原先对 hok 完全不可见：不列、`cleanup -k` 不删、`cache rm <app>` 不删，只有 `cache rm *` 能删）；② 坏安装（current 解析不出）的缓存也删（`cache::remove_all_for_app`，官方 `-Exclude "$app#$null#*"` 匹配不到任何东西 ⇒ 该 app 缓存全清；版本目录仍不动 = 有意偏离）；③ `cleanup -k` 补删除汇总行（**hok 扩展**，零删除也打） | `ff05b80`（cache 可见性）/ `cb75ba4`（cleanup 汇总 + 坏安装），worktree `.worktrees/s-cleanup-cache`，分支 `fix/cleanup-cache-names`（base = 主线 `cdcfab1`）→ **已线性 ff 进 `refactor/edition-2024`（tip `cb75ba43`，2 提交，无 merge commit；`worktree-merge.ps1 -VerifiedRef cb75ba4`：无需 rebase、提交数/主题/6 文件集守恒、落地树与受测 ref 逐字节相同）** | 门禁：worktree `cargo test -p libscoop` 786 passed / 0 failed、`cargo test -p hok` 全绿、`cargo clippy --workspace --all-targets --exclude innospect --no-deps -- -D warnings` 干净、改动文件 `rustfmt --edition 2024 --check` 干净；隔离 `SCOOP` 根端到端 5 条（列表 2→6 个文件、`cleanup * -k` 删 3 个 + 汇总行、broken app 缓存被清、当前版本缓存 + sidecar 保留、二次运行 0 行）；真机 `$env:SCOOP` 实跑一次（16 个旧版本目录 + 6 个过期缓存 ≈156 MiB，当前版本缓存 19 文件 / 2 GB 按设计保留）。**同批清掉 `pending-tasks` 挂账**「CJK 包名缓存文件被拒」。⚠️ 坑：`cargo test -p libscoop <filter>` 会踩既有 flaky `manifest_cache::tests::serving_a_stale_cache_warns_once`（clean HEAD 同 filter 同样失败 ⇒ 与本次无关），验收以全量 `cargo test -p libscoop` 为准。**追加 `7b92635`（同日，worktree `.worktrees/s-cleanup-parts`）**：`.parts` 残留分片也扫（`remove_incomplete` 补扫 `<缓存名>.parts` 目录 + 新汇总行 `cmd.cleanup_incomplete_removed`；官方非递归 `Remove-Item` 同样清不掉，属有意扩展）。另：本批顺带查明的既有 flaky 用例（`manifest_cache::tests::serving_a_stale_cache_warns_once`，进程级 once 闩锁 + 每会话 sink 错位，实测并行 2/25 失败、串行 0/12）**不在本会话修**，已挂 `pending-tasks` 并交新会话引导词 |

---

### S7 + S14 落地记录（2026-09-13）

> **范围与决策（用户确认）**：T1+T2 全做；并行切分 = **P2（1 个 worktree 串行）**；`status` **保留** `Source` 列与 `:: Status of installed apps` 分节标题；`shim` 无子命令**保留**默认 list；`upgrade -f` 改为 **force**。
> **worktree / 分支**：`.worktrees/s7s14-low-risk` / `fix/s7s14-low-risk`，base `cf1baec`（线性后继，`--ff-only` 可落地）。
> **完整报告**：`.agents/docs/active/audit/s7-s14-investigation-2026-09.md`（逐项对照、`status` 实测前后、config 别名核查、合并就绪核查）。

**做了**：①标志/命名对齐 —— `--skip-hash-check` 唯一拼写并删 `--no-hash-check` 别名 + 内部 `skip_hash_check`/`SkipHashCheck`；`--ignore-failure` 去掉短选项（原 `-f` 与官方 force 冲突）+ 内部 `ignore_failures`/`IgnoreFailures` + 帮助写明可持久设置；`-D/--ignore-cache` → `-k/--no-cache` + 内部 `no_cache`/`NoCache`；`upgrade -f` = force；S4 设置键统一为 `ignore_failures`/`no_color`/`script_engine` 并**删除 `config.rs` 的 serde 别名**。②`status` 列名 `Missing Dependencies`、缺依赖 ` | `、Info 词 `, `、全好文案 `Everything is ok!`。③`list` 表头回显查询、Info 词按命令分语境（`status_info_word` 加 `InfoContext` 单入口）、新增 `Updated` 列。④`search` 无查询列出全部。⑤`download -f` 打印 `Cache is being ignored.`。⑥`reset <未安装>` exit 0。⑦`update` 逐应用 `app: ver (latest version)` + 结尾提示、`--no-cache` 无 `<app>` 报错 exit 1。⑧`install` 已安装提示带版本 + `Use 'hok update <app>'`。⑨`bucket` 列序 Name/Source/Updated/Manifests、known bucket 排序、`known` 裸名称、known 名大小写不敏感。⑩`cache rm` 逐文件输出、`cleanup` 逐版本输出。

**⚠️ 破坏性变更（发布说明 / README 需登记）**：
1. `--no-hash-check` 不再可用，只剩 `--skip-hash-check`（`-s` 不变）；
2. `-D`/`--ignore-cache` 不再可用，改为上游拼写 `-k`/`--no-cache`；
3. `--ignore-failure` 不再有短选项 `-f`（`update`/`upgrade`/`download` 的 `-f` 都是 force；`upgrade -f` 语义由 ignore-failure 改为 force）；
4. 设置键旧拼写不再可读：`ignore-failures` / `no-color` / `script-engine` / `ignore_failure` / `cachePath` / `globalPath` / `rootPath` / `lastupdate` 等，配置文件中出现会被**静默忽略**（该设置回落默认值）。

**有意偏离（登记为不改）**：`status` 的 `Source` 列、attention 页脚、`:: <标题>` 分节标题；`list` 的升级箭头 `→ <new>`、计数页脚、多查询 OR、`-V/-e/-u/-H`；`search` 的输出形态与多词 OR；`bucket list -k` 与 `bucket rm` 多名称；`shim` 无子命令默认 list、无 `alter`。~~`install`/`update` 成功的 `Everything is ok!`~~（**2026-09-13 晚改为删除**：官方 install/uninstall/update 无此页脚，只有 `scoop status` 有 ⇒ 从"有意偏离"转为"对齐"；`status` 那处保留）。

**验证**：隔离 `SCOOP`/`HOME` 根实测（`status` 前后对比见报告 §9.1；`cache rm` 逐文件、`cleanup` 逐版本与 `-v`、`list` 的 Updated 列、`shim list`/`info` 表格与 `shim info ghost` exit 3）；`cargo test --workspace` 仅 2 条 ACL 环境性失败（clean HEAD 同样失败）；`--test i18n_keys|cli_help_i18n|exit_codes` 全绿；`clippy --workspace --all-targets --exclude innospect --no-deps -- -D warnings` 干净；改动文件级 `rustfmt --config skip_children=true --check` 干净（**勿缺 `skip_children`**，否则从 `lib.rs` 递归进子模块会误报无关文件）。

**落地**：`worktree-merge.ps1 -Worktree .worktrees/s7s14-low-risk -VerifiedRef aed0734` → 12 提交线性 ff 进 `refactor/edition-2024`（tip `aed0734`，无 merge commit）；主树遗留的 5 个未提交文件（上一会话：`.gitignore` 的 `!.agents/docs/` + 4 处 `docs/`→`.agents/docs/` 路径修正）在 ff 前 stash、ff 后 `pop` 原样恢复（无冲突）。

**后续项（记一笔）**：`install`/`update` 的 `--independent` 短选项应对齐官方 `-i`（现为 `-I`），描述改用官方 `Don't install dependencies automatically`。

---

## 7. 对话编排（把 S1–S14 收敛成若干个对话）

**编排策略**

- 一条对话 = 一个**决策主题**（同一批拍板点、同一片代码）；
- 纯本地、无跨命令决策的小项可以合并进一条对话（**提交仍按主题分组**）；
- 跨命令的决策（退出码体系、批量失败策略）必须独占一条——它会改变其它条目的判定；
- hok 独有命令（`checkver`/`checkhashes`/`checkurls`/`formatjson`/`missing-checkver`/`virustotal`/`ci-auto-pr`）彼此独立，**一条命令一条对话**，且是否要做取决于你是否真在用它们。

### 7.1 推荐批次：常规 8 条对话

| # | 覆盖 | 主题 | 依赖 | 开场 prompt（复制即用） |
|---|---|---|---|---|
| 1 | S1 + S13 | 退出码与批量失败策略口径（先出官方退出码映射表，再定是否对齐；批量行为随口径落地） | 无（**先做**） | `按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S1+S13 执行：先出官方退出码映射表与候选方案，等我确认后再改。` |
| 2 | S2 | `alias` 可用性（生成脚本 / 内部分发 / 明确记录语义） | 无 | `按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S2 执行：先给 alias 可用性的候选方案与代价，等我确认。` |
| 3 | S3 | `export`/`import` 与官方 Scoopfile 互通 | 无 | `按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S3 执行：先给 schema 互通方案（采纳官方 / 仅 import 兼容 / 双格式），等我确认。` |
| 4 | S4 + S12 | `config` 组：键策略、单键读取、`unset`/`rm`、`proxy none`，并顺手核实 `ignore-failures` 默认显示疑似 bug | 无 | `按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S4+S12 执行：先核实 ignore-failures 是否真 bug，并给 config 组的方案选项，等我确认。` |
| 5 | S5 | `checkup` 语义决策（对齐环境检查 / 改名 / 两者并存） | 无 | `按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S5 执行：先给三个语义方案的影响面（含破坏性变更），等我确认。` |
| 6 | S6 | `cleanup`/`cache` 数据安全（`-k` 缓存、`cache rm` 匹配、admin 检查、`-a -g`） | 无 | `按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S6 执行：先出数据安全影响与方案，等我确认。` |
| 7 | S7 + S14 | 低风险批量：文案 / 列名 / 列序 / 默认值 / 参数 / 小退出码（含 `bucket`、`shim`） | 无 | `按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S7+S14 执行：先列清单与测试快照影响，等我确认后按命令分组提交。` |
| 8 | S8 | 路径解析类：`which` / `prefix` / `cat` / `home` | 无 | `按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 S8 执行：先出 which 目标解析与 Get-Manifest 等价入口方案，等我确认。` |

> 对话 1 完成后回来的结论会影响对话 6/7/8 中"退出码不同"那几行的处置（对齐 or 登记为有意偏离），但不阻塞它们的其余部分。

### 7.2 精简版：5 条对话（只做最高价值）

1 → 2 → 3 → 4 → 5（即上表前 5 条）。S6/S7/S8 以及全部 hok 独有命令暂缓；适合"先把功能性缺陷和口径定下来"，日常使用体感提升最明显。

### 7.3 hok 独有命令：按需，每条命令一条对话

| # | 覆盖 | 命令 | 建议优先级 | 备注 |
|---|---|---|---|---|
| A | S11-formatjson | `formatjson` | **高** | 含实际 bug：子命令名被 kebab 化为 `format-json`，`hok formatjson` 报 unrecognized subcommand |
| B | S10 | `checkhashes` | 中 | 缓存策略 + 回写安全（注意 CRLF/末尾换行约定） |
| C | S9-1 | `checkver`：URL 快捷分支与提取链 | 中 | 建议先出"真实 bucket 会踩到"的优先级清单 |
| D | S9-2 | `checkver`：autoupdate 变量表与属性替换 | 中 | 与 C 可先后 |
| E | S9-3 | `checkver`：hash 提取链（`lib/autoupdate.ps1` 全套兜底） | 中 | 最大的一块，独立成对话 |
| F | S11-missing-checkver | `missing-checkver` | 中 | `-s/--supported` 与官方 `-SkipSupported` **语义相反**，先定口径 |
| G | S11-checkurls | `checkurls` | 低 | HEAD vs GET、3xx 判定 |
| H | S11-virustotal | `virustotal` | 低 | 与对话 1 的退出位掩码联动 |
| I | S11-ci-auto-pr | `ci-auto-pr` | 低 | 变更判定机制差异（字节快照 vs `hub diff`） |

### 7.4 编排规则

1. **一条对话只改一个主题**；同一对话内若涉及多组改动，提交按主题分组（一主题一 commit）。
2. **每条对话开场先调研**（全局 `investigate-first`），报告含"该不该做"的判定；报告是候选清单，不追求清空。
3. **不要在同一对话里连做两条**（如 alias 做完接着做 export/import）——宁可收尾提交后另开，避免上下文互相污染与提交混主题。
4. **可并行**：对话 2–8 之间无依赖，理论上可同时开；但**同一条对话不要跨仓库状态并行改同一批文件**（`config.rs`、`manifest.rs` 易冲突），建议串行或至少确认无文件重叠。并行时**必须**按全局 `worktree-parallel`：一会话一 worktree、收尾 rebase 到主线当前 tip 后 `--ff-only` 线性落地（禁 merge commit）、汇总型热点文件只在主树小步改（`.agents/**` 其余部分已入库）。
5. **收尾固定动作**：更新本计划 §6 进度、报告对应行状态、`.agents/skills/scoop-gap-audit.md`；代码分支按全局 `worktree-parallel` §5 线性合并。

---

## 8. 剩余工作（新会话交接，2026-09-13 立）

> **本文是给下一条对话的入口。** 上一轮（S1–S14 + P0 + D4 + `-d` 护栏）已全部落地，本节的四项是**尚未开工**的余项。

### 8.1 已完成状态（供新会话确认起点）

当时主线 `refactor/edition-2024` 的 tip = **`8a356d0`**（= `8ad7d080` 之后仅一个 `chore(docs)` 提交，把残留的 `docs/` → `.agents/docs/` 路径引用补完并让 `.agents/docs/` 保持"可见但未跟踪"），全部线性落地、**零 merge commit**。（**更新 2026-09-13**：此后又线性落地 update/upgrade 对齐、运行中进程输出 + `ignore_running_processes`、`Everything is ok!` 页脚删除、**下载默认不发 Referer + 「全在缓存中」误报修复**（`cf260d9`/`4748ae1`）⇒ **更新 2026-09-13 晚：R2 与 R1 亦已线性落地**（R2 → `9b672207`，3 提交；R1 → `43795484`，5 提交；均 `--ff-only`、零 merge commit）⇒ **当前 tip = `43795484`**。）已闭环：S1+S13、S2、S3、S4(+S12)、S5、S6、S7(+S14 输出类)、S8、S9、S10、S11 第一批（`formatjson` + `missing-checkver`）、S14（bucket/shim）、**P0**（`autoupdate.hash` 的 `mode: github`）、**D4**（下载请求头统一）、**`-d` 写盘护栏**。计划 §6 表已逐行回填；细节与结论全在 `.agents/skills/scoop-gap-audit.md`（含各条"有意偏离"与"勿踩坑"）。

### 8.2 余项（按用户使用面排序）

| 编号 | 主题 | 建议 | 依据 |
|---|---|---|---|
| ✅ **R1** | **`hok checkver` 挂起** | **已完成（2026-09-13）** | 根因 = `package/checkver.rs:383` 的 `.buffer_unordered(pending.len())` 在 `pending` 为空时容量 **0**，futures 零容量语义下源流永不被 poll ⇒ `Pending` 无 waker ⇒ 永久挂起；触发面含**任何 `--version`**。修法 `.max(1)` + 回归（含 `--version`）+ 清陈旧 TODO + 同类站点 `debug_assert!`，5 提交 → 主线 **`43795484`**（`--ff-only`，零 merge commit）。详见 `.agents/docs/active/plan/pending-tasks.md` §8.4 S1 |
| ✅ **R2** | **缺环境变量 panic** | **已完成（2026-09-13）** | 归因经实测更正：写链上空路径来自 **`hok_config_path()`**（非 `scoop_config_path()`），且与 `APPDATA` 无关。修法 = `fs.rs:63/115` 降级为 `Error::Io(InvalidInput)` + `init_default` 跳过空路径 + 空串 `SCOOP`/`SCOOP_GLOBAL` 视为未设置（对齐官方 `core.ps1:1291-1301`），3 提交 → 主线 **`9b672207`**（`--ff-only`，零 merge commit）。详见 `.agents/docs/active/plan/pending-tasks.md` §8.4 S2 |
| **R3** | **解析失败静默消失**：`package/query.rs:120` / `:285` 的 `filter_map` 把清单解析失败变成"**找不到软件包**" | **做（设计题，必须先过决策门）** | 这是 P0 难定位的**根因**（解析层严格 + 查询层静默）。要定"搜索/安装遇到不可解析清单该报错、警告还是计数"，会改单入口的错误传播策略，风险最高 |
| **R4** | **`ci-auto-pr` 对齐**（S11 余项） | **做（用户在 CI 里用它更新清单）** | 注意它**写盘并提交**，受 `-d` 写盘护栏约束（当前 `-d` 默认 `bucket`，应改为"写盘时要求显式目标"）；审计 §7.7 的差异点：变更判定机制（字节快照 vs `hub diff`） |
| — | `checkurls`（S11 余项） | **暂缓**（用户用得少） | 只读扫描，审计差异多为输出/边界。**追加待办（随 checkver/checkurls 专题一并处理）**：`HeaderProfile::CheckUrls` 跟随下载路径**不发 Referer**（现状按官方 `bin/checkurls.ps1:60` 发 ⇒ 反盗链 CDN 上出现假坏链，腾讯/阿里云同款）；`Checkver`/`Autoupdate` 保持官方口径不动 |
| — | `virustotal`（S11 余项） | **不做**（用户基本不用） | 含退出位掩码那套；登记为不修 |
| **R5** | **恢复 `-c/--cascade`**：hok 曾有该 flag，`5f2532e`（eventloop 统一）把它连同 `options.push(SyncOption::Cascade)` 一起删掉，此后 `SyncOption::Cascade`/`NoDependentCheck` **无任何构造者** ⇒ 级联卸载（"删除不再被依赖的依赖"）在生产**不可达**，`resolve_cascade` 只被 4 个单测调用 | **做（单开，属新功能 + 有意偏离官方）** | `.agents/skills/scoop-gap-audit.md`、`crates/libscoop/src/package/resolve.rs:304-393`；恢复后 held 依赖按新政策**恒报错**（要求先 `hok unhold`，无 bypass）；官方 `scoop-uninstall.ps1:5-7,18` 只有 `-g/-p`，无对应能力 |
| **R6** | **`PRIVATE_HOSTS` 注入语义 = 替换（官方）而 hok 是 append**：`internal/network.rs:188-192` 用 reqwest `.header()`（`reqwest-0.12.28/src/async_impl/request.rs:223` 是 append），官方 `$wreq.Headers[$key] = $value` 是替换 ⇒ 同名头（Referer/User-Agent/Authorization）会发两条 | **做（用户 2026-09-13 定：随 checkver/checkurls 专题一并处理）** | reqwest 已带替换入口：`RequestBuilder::headers(map)` → `util::replace_headers`（`request.rs:239-242`、`src/util.rs:50-65`，同名 `Occupied` 直接 `insert`）⇒ 改法≈12 行。三个坑：①**别改成静默跳过**——无效头名/值现在会让请求报错（loud），应保持；②官方 `checkver.ps1:240-246` 自己是"先设 PRIVATE_HOSTS 再 `Headers.Add('Referer')`"⇒ 官方在 checkver 上**也**出重复头，**不要照抄这个 bug**（hok 统一替换会比官方更干净，属有意偏离）；③官方只在 download/checkurls/checkver 注入（autoupdate 不注入），hok 全 profile 注入是既有有意偏离，保持 |

### 8.3 建议的会话切分

- **Wave 1（2 条，互不重叠，可同时开）**：`R1`（`.worktrees/r1-checkver-hang` / `fix/r1-checkver-hang`）、`R2`（`.worktrees/r2-env-panic` / `fix/r2-env-panic`）。
- **Wave 2（Wave 1 落地后再开）**：`R3`（设计题，单开）、`R4`（单开；若想让父会话少做一次收口，也可与 R3 分属不同分支但同日开）。
- 每条都必须：`investigate-first`（先调研报告 + 选项 + 等确认）→ `worktree-parallel`/`hok-worktree-parallel` → `hok-rust-gate-practice` → `proactive-review` → 签名提交 → 父会话 `worktree-merge.ps1` 线性 ff。

### 8.4 新会话开场 prompt（复制一行）

```
按 .agents/docs/active/plan/command-deviation-fix-plan-2026-09.md 的 §8 执行：先读 §8 与 .agents/skills/scoop-gap-audit.md 确认起点，
然后为 §8.2 的 R1 + R2 各派一个子代理（自带 worktree，先调研出报告、等我确认后再改），你在父会话负责收口（串行 rebase + --ff-only + 落地树重跑门禁）与文档回填。
```

### 8.5 交接要点（新会话务必先读）

1. **沙箱限制**：本机 msys 的 `sh.exe`/`true.exe` 起不来 ⇒ **`git rebase --continue` 走不通**（`GIT_EDITOR`/`core.editor` 全无效）；用 `cherry-pick -n` + `commit -C` 逐提交重放。提交信息一律 `-F <文件>`（PowerShell 会展开反引号与 `$`）。**有未提交改动时绝不 `reset --hard`**。
2. **收口脚本的坑**：`worktree-merge.ps1` 在 worktree 处于 **detached HEAD** 时会失败并可能把 `.agents/**` 误暂存进主树索引（用 `git reset` 复原）；`-VerifiedRef` 的判据是"与受测树逐字节相同"，base 落后时必然报 `ERR the rebased tree differs`（预期，不是漂移）。
3. **汇总型热点文件只在主树改**：`pending-tasks.md`、本文件、`command-deviation-audit-2026-09.md`、`scoop-gap-audit.md`。`.agents/**` 自 2026-09-26 已入库、子代理能看见，但这四个**禁止子代理写**；回填由父会话做（写入前重读、行级替换）。
4. **本机出站 HTTPS 不可用**（直连与代理 `127.0.0.1:7890` 均 TLS 失败，纯 HTTP 可用）⇒ 任何"真实源冒烟"都做不了，只能用本地夹具 + 回环测试。
5. **并发上限建议 2–3 条**：凡是碰 `locales/*.yml`、`README.md`、`src/cmd/mod.rs` 的分支必然冲突，且冲突要父会话手工重放，成本高。

