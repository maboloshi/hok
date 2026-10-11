# 未开工会话卡片（H3 / H4 / R5 / R6 / PB1）

> 从待办总表迁出的派活卡片原文；本表是复制即用的 prompt，不是文档。
> 待办状态见 `.agents/docs/active/plan/pending-tasks.md`。

## 共同前置（五张卡片共用）

- **工作树纪律本体**：[`hok-worktree-parallel`](../../../skills/hok-worktree-parallel.md)——一会话一 worktree、base = 主线**当前** tip、`--ff-only` 线性落地、禁 merge commit、收尾四件套。
- **派活纪律本体**：[`hok-agent-orchestration`](../../../skills/hok-agent-orchestration.md)——文件 + 语义重叠矩阵 → 并发封顶 2–3 → prompt 契约 → 父会话收口。
- **主线 tip 自己取**：`git rev-parse --short $(git config worktree-merge.mainbranch)`。总表与本文件里出现的任何 sha 都是历史快照，**别当待办**。
- 先加载的技能：`investigate-first`、`hok-worktree-parallel`、`hok-agent-orchestration`、`hok-rust-gate-practice`、`git-commit-practice`、`proactive-review`、`no-patchwork-fixes`、`token-efficient-processing`。
- **范围纪律**：只用本卡片约定的 worktree 与分支；不合主线、不碰别会话的 worktree、不派生下级子代理。**汇总型热点文件**（[`pending-tasks.md`](../../active/plan/pending-tasks.md)、[`command-deviation-audit-2026-09.md`](../../active/audit/command-deviation-audit-2026-09.md)、[`scoop-gap-status.md`](../../active/audit/scoop-gap-status.md)）**只在主树改**，子代理禁止写；`.agents/**` 其余部分已入库，可正常改、正常提交；[`scoop-gap-audit`](../../../skills/scoop-gap-audit.md) 只是方法论，不再回填落地记录。
- **门禁与沙箱坑的本体**：[`hok-rust-gate-practice`](../../../skills/hok-rust-gate-practice.md)（clippy / rustfmt 命令、ACL 环境噪声的判据）与 [`hok-worktree-parallel`](../../../skills/hok-worktree-parallel.md) 的「本仓事实」（先 `cargo build -p hok-shim`、`rebase --continue` 走不通、提交走 `-F`、出站 HTTPS 与 `cargo fetch`）。**不要在这里另抄一份命令**，以那两处为准。
- **必须记住的硬动作**：新 worktree 先 `cargo build -p hok-shim`；提交信息一律 `git commit -F <文件>`；有未提交改动时绝不 `reset --hard`；落地判据是 `cargo test --workspace` **0 failed**（`permission::grant_users_write_inherit_*` 两条 ACL 失败属沙箱环境噪声）；签名提交直接提交，**不要**设 `sandbox_permissions`（审批已关闭，会被自动拒绝）。
- **worktree / 分支名**：卡片里给的是沿波次命名的**建议名**；与现存 worktree 冲突时可直接改名，**复用同名 worktree 前先确认它的工作区状态**（可能留着上次会话的改动）。
- **门禁 / 决策纪律**：任何"新功能 / 行为变更 / 重构 / 去重 / 选型"先出调研报告（证据 + 候选含"不做" + 影响面 + 推荐）→ **停下等用户确认** → 再动代码；交回的是**可复跑的命令 + 前后对照**，不是"已修复"。允许用证据反驳计划——先重跑再下结论。

## H3 清单解析失败静默消失（设计题，单开）

- **依据**：总表 `H3` 行；设计报告已出 —— [`h3-parse-visibility-design-2026-09.md`](../h3-parse-visibility-design-2026-09.md)（本机 4 个 bucket 的 21710 个清单里 80 个被 hok 拒收、**80/80 官方接受**；推荐 O2a+O3；**待拍板 D1–D5**：输出形态 / 退出码 / 缓存侧 / 80 缺口另立 / 文案）。P0-4（`autoupdate.hash.mode: github`）难定位的**根因**。
- **目标**：定"搜索 / 安装遇到不可解析清单"的可见性策略并落地。
- **决策门（必须先报告、禁先改）**：候选 = 报错 / 警告 / 计数 / 保持静默；影响面覆盖全部清单扫描路径（`search` / `list` / `install` / `info` / `checkver` / `checkurls` / `missing-checkver`）。**量化要求**：用本机 4 个真实 bucket 统计不可解析数量与受影响命令面（设计报告已给基线：4 个 bucket / 21710 个清单 / 80 个被拒，落地时重跑确认；参照 P0-4 那次的量化做法）。设计报告已列出 D1–D5，直接按它汇报并请拍板，不要重新设计一遍。
- **硬性禁区**：不改 `internal::json` 的宽容度契约（11 条三方裁决）；不把"严格解析"改回"warn→null"。
- **验收**：①决策门确认的策略在**所有**扫描路径一致；②隔离夹具（故意放坏清单）逐命令实测；③`cargo test --workspace`。
- **提交拆分**：① `fix(query): surface manifest parse failures instead of dropping them` ② `test(query): pin the visibility policy` ③（如需）`i18n`。
- **worktree / 分支**：`.worktrees/w2a-parse-visibility` / `fix/parse-visibility`（已裁决：沿用波次命名一代）。
- **开场 prompt**：

  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 H3 行执行：这是设计题，单开。
  先读该文件的 H3 行、设计报告 .agents/docs/reports/h3-parse-visibility-design-2026-09.md
  （设计已做完，D1–D5 待拍板），以及 .agents/skills/hok-agent-orchestration.md 的 §3。
  开 worktree .worktrees/w2a-parse-visibility（分支 fix/parse-visibility）；base=主线当前 tip。
  给出"可见性策略候选（报错 / 警告 / 计数 / 保持静默）+ 真实 bucket 量化影响面"报告，
  含 D1–D5 各选项的代价与风险，等我确认后再改代码。
  硬禁区：不改 internal::json 的宽容度契约、不把严格解析改回 warn→null。
  ```

## H4 `ci-auto-pr` 对齐

- **依据**：总表 `H4` 行；[`command-deviation-audit-2026-09.md`](../../active/audit/command-deviation-audit-2026-09.md) 的 §7.7 差异点（变更判定：hok 字节快照 vs 官方 `hub diff`）。现状复核：`src/cmd/auto_pr.rs` 的 `-d` 仍是 `default_value = "bucket"`，且不在 `write_dir` / `scan_dir` 单一入口的使用者之列（现使用者只有 `checkhashes` / `checkver` / `formatjson`）。
- **目标**：对齐审计 §7.7 的差异点。
- **决策门（必须先报告、禁先改）**：①`-d` 当前默认 `bucket`，按 `-d` 写盘护栏的统一规则应改为"**写盘时必须显式目标**"（破坏性，需确认）；②变更判定机制（hok 字节快照 vs 官方 `hub diff`）是否对齐。
- **硬性禁区**：不改 `-d` 护栏工具函数 `write_dir` / `scan_dir` 的既有语义（复用即可）；不在 CI 环境做真实 push 测试。
- **验收**：①隔离环境干跑（无变更 / 有变更两条路径）；②缺 `-d` 时 exit 1 + 诱饵树未动；③`cargo test --test exit_codes --test cli_help_i18n`。
- **提交拆分**：① `fix(ci-auto-pr): require an explicit target when writing` ②（如需）`align change detection` ③ `i18n/docs`。
- **worktree / 分支**：`.worktrees/w2b-ci-auto-pr` / `fix/ci-auto-pr`（已裁决：沿用波次命名一代；该目录当前已存在，复用前先确认其工作区状态）。
- **排期约束**：与 H3 **错开 `locales/*`**，勿同日开。
- **开场 prompt**：

  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 H4 行执行。
  先读该文件的 H4 行与 .agents/docs/active/audit/command-deviation-audit-2026-09.md 的 §7.7。
  开 worktree .worktrees/w2b-ci-auto-pr（分支 fix/ci-auto-pr）；base=主线当前 tip。
  先给两个决策点的候选与影响面，等我确认后再改：① `-d` 当前默认 bucket，按 -d 写盘护栏的统一规则
  应改为"写盘时必须显式目标"（破坏性）；② 变更判定机制（hok 字节快照 vs 官方 hub diff）是否对齐。
  注意它写盘并提交，测试用隔离环境干跑，不做真实 push；与 H3 错开 locales/*，勿同日开。
  ```

## R5 恢复 `-c/--cascade`

- **依据**：总表 `R5` 行。`SyncOption::Cascade` / `NoDependentCheck` **无任何构造者** ⇒ 级联卸载在生产**不可达**；`resolve_cascade` 只被 4 个单测调用（枚举定义 `package/resolve.rs:219`，读取 `sync_remove.rs:33/99`；成因是 `5f2532e` 连同 `options.push(SyncOption::Cascade)` 一起删掉、提交信息未提）。
- **目标**：恢复级联卸载（"删除不再被依赖的依赖"），让 `SyncOption::Cascade` 在生产可达。
- **决策门（必须先报告、禁先改）**：①flag 形态（恢复 `-c/--cascade` 到 uninstall vs 保持删除）；②held 依赖**恒报错**（要求先 `hok unhold`，无 bypass）确认；③官方无此能力 ⇒ 登记为有意偏离。
- **硬性禁区**：**不许顺手删掉** `resolve_cascade` / `NoDependentCheck`；`NoDependentCheck` 目前同样无构造者，一并决策。
- **验收**：①三件套场景（依赖仍被需要 → 保留；不再被需要 → 级联删除；held → 报错）逐条实测；②恢复 / 新增单测；③`cargo test -p libscoop -p hok`。
- **提交拆分**：① `feat(uninstall): restore cascade removal` ② `test(resolve): pin the cascade decisions` ③ `i18n/docs`。
- **worktree / 分支**：`.worktrees/w3a-cascade` / `feat/cascade-uninstall`（已裁决：沿用波次命名一代）。
- **开场 prompt**：

  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 R5 行执行。
  先读该文件的 R5 行，以及 crates/libscoop/src/package/resolve.rs 的 resolve_cascade
  （旧卡片另给了行段 :304-393，落地时以实际为准；该函数已被 4 个单测覆盖）。
  开 worktree .worktrees/w3a-cascade（分支 feat/cascade-uninstall）；base=主线当前 tip。
  先给定 flag 形态与 held 政策的候选，等我确认后再改。
  硬禁区：不许顺手删掉既有级联逻辑（SyncOption::Cascade / NoDependentCheck 目前无构造者，
  但 resolve_cascade 已被 4 个单测覆盖）。
  ```

## R6 `PRIVATE_HOSTS` 替换语义 + `CheckUrls` Referer

- **依据**：总表 `R6` 行（含"`HeaderProfile::CheckUrls` 不发 Referer"追加待办）。官方 `$wreq.Headers[$key] = $value` 是替换；hok 的 `internal/network.rs` `apply_headers` 用 reqwest `.header()`（append）⇒ 同名头（Referer / User-Agent / Authorization）会发两条；reqwest 已有替换入口 `RequestBuilder::headers(map)` → `util::replace_headers`。
- **目标**：①同名头改为**替换**（对齐官方 `$wreq.Headers[$key] = $value`）；②`HeaderProfile::CheckUrls` 不再发 Referer（跟随下载路径的既有决定）。
- **决策门（必须先报告、禁先改）**：①hok 统一替换会比官方更干净（官方 checkver 自身也出重复头，`checkver.ps1:240-246`）——确认为**有意偏离**；②`checkurls` 结果会变（反盗链 CDN 上从"假坏链"变正常）——破坏性需确认。
- **硬性禁区**：保持"无效头名 / 值会让请求报错"（**别改成静默跳过**，现在是 loud）；请求头**唯一入口**仍是 `ScoopRequest` / `HeaderProfile`，**别新开第二条注入路径**；`Checkver` / `Autoupdate` 保持官方口径。
- **验收**：①本地夹具断言同名头**只出现 1 条**；②`count == 1` 的新测试；③`checkurls` 对本地夹具的假坏链用例转正常；④`cargo test -p libscoop -p hok`。
- **提交拆分**：① `fix(network): replace private-host headers instead of appending` ② `fix(network): stop sending a Referer from the checkurls probe` ③ `test`。
- **worktree / 分支**：`.worktrees/w3b-private-hosts` / `fix/private-hosts-headers`（已裁决：沿用波次命名一代）。
- **开场 prompt**：

  ```
  按 .agents/docs/active/plan/pending-tasks.md 的 R6 行执行（合并会话：PRIVATE_HOSTS 替换语义 + CheckUrls Referer）。
  先读该文件的 R6 行。
  开 worktree .worktrees/w3b-private-hosts（分支 fix/private-hosts-headers）；base=主线当前 tip。
  先给两个候选与影响面，等我确认后再改：① 同名头由 append 改 replace（官方 $wreq.Headers[$key] = $value；
  hok 统一替换会比官方更干净，属有意偏离）；② HeaderProfile::CheckUrls 跟随下载路径不再发 Referer
  （反盗链 CDN 上的"假坏链"会变正常，属行为变化）。
  硬禁区：别改成静默跳过无效头（现在是 loud）；请求头唯一入口仍是 ScoopRequest/HeaderProfile，
  别新开第二条注入路径；Checkver/Autoupdate 保持官方口径。
  ```

## PB1 私有 Scoop bucket SSH

- **依据**：[`private-bucket-plan-2026-09.md`](../../active/plan/private-bucket-plan-2026-09.md) §4（P1a SSH 凭据 / P1b known_hosts TOFU / P2 token 注入 / P3 README），§5 已给 4 个提交拆分；`private-bucket-plan-2026-09.md` 的 4 个提交拆分；总表 `PB1` 行。现状：**私有 HTTPS 今天就能用**（libgit2 → 系统凭据助手，非沙箱实测通过）；**私有 SSH 完全不可用**（缺凭据分支 + 缺 hostkey 校验）。前置技能：`investigate-first`、`hok-code-organization`、`no-patchwork-fixes`。
- **目标**：按方案补齐私有 bucket SSH 支持（凭据 + hostkey TOFU + token 注入 + 文档）。
- **决策门（必须先报告、禁先改）**：先确认"补齐 SSH"路线（vs 只关 `git2` 的 ssh feature 并在 `bucket add` 显式报错登记 gap，后者不满足官方 SSH 支持，需与 P1a/P1b 一起定夺）。
- **硬性禁区**：无（两代卡片均未写；方案本身的取舍以 `.agents/docs/active/plan/private-bucket-plan-2026-09.md` 为准）。
- **验收**：`cargo test -p libscoop`；**非沙箱手工三验**（沙箱下 TLS / Schannel 不可用）：GitHub 私有库 HTTPS + token、`private_hosts` 自定义头、SSH 首次写 `~/.ssh/known_hosts` + 篡改后报 host key changed。
- **提交拆分**：见 [`private-bucket-plan-2026-09.md`](../../active/plan/private-bucket-plan-2026-09.md) §5 的 4 个提交（P1a / P1b / P2 / P3）。
- **worktree / 分支**：`.worktrees/w4-ssh-buckets` / `feat/ssh-buckets`（已裁决：沿用波次命名一代）。
- **开场 prompt**：

  ```
  按 .agents/docs/active/plan/private-bucket-plan-2026-09.md 的 §4 执行（P1a SSH 凭据 / P1b known_hosts TOFU /
  P2 token 注入 / P3 README），提交拆分见该文件 §5。
  先读 .agents/docs/active/plan/pending-tasks.md 的 PB1 行。
  开 worktree .worktrees/w4-ssh-buckets（分支 feat/ssh-buckets）；base=主线当前 tip。
  先确认路线（补齐 SSH vs 只关 git2 的 ssh feature + 在 bucket add 显式报错并登记 gap），
  等我确认后再改。验收要求非沙箱手工三验（沙箱内 TLS 不可用）。
  ```
