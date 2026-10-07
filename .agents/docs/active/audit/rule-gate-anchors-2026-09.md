# 规则载体的闸门锚点审计（2026-09）

Status: active
Updated: 2026-10-04

> **起因**：devlog 触发在 8 个文件里各写一遍、措辞漂成三种（"每条会话" / "每项修完" / "新增条目"），
> 本会话被这个缝隙绊到；`c186cf5` 已把触发收口进 `.agents/docs/AGENTS.md` R7。这次把**整档闸门**扫一遍。
> **方法**：只读通读 20 个仓内规则载体（根 `AGENTS.md` + 3 层 `AGENTS.md` + 16 个技能）与 11 个机器本地
> 全局技能，逐条判「闸门 vs 知识」与「复述 vs 引用」；子代理分片扫描，凡要写进常驻层（根 `AGENTS.md`）
> 的结论由父会话**逐条复核**，复核结果见 §5。
> **分层判据**已固化进 `.agents/AGENTS.md` → 「规则分层」。

## 1. 闸门登记表（审计后状态）

| 闸门（时机 → 动作） | 本体 | 常驻锚点（根 `AGENTS.md`） | 判定 |
|---|---|---|---|
| 动手前：调研 → 报告（证据+选项+推荐）→ 等确认 | 全局 `investigate-first` | Gates → Before implementing | ✅ |
| 重构/去重前：先问"解决什么真实问题" + 量化门槛 | 全局 `proactive-review` §7（量化门槛）+ `hok-code-organization`（真实问题 + P2-6 判例） | Gates → Before implementing（本次补） | ✅ |
| 提交前：整体 review（行为锚点/边界/架构/收益） | 全局 `proactive-review` + `hok-proactive-review` | Gates → Before committing | ✅ |
| 暂存前：一主题一 commit / `-c core.autocrlf=false` / `git commit -F <file>` | 全局 `git-commit-practice` | Gates → Before staging（本次补） | ✅ |
| 收尾：fmt / clippy / test 确定性闭环 | 全局 `rust-deterministic-gate` + `hok-rust-gate-practice` | Gates → Deterministic wrap-up | ✅ |
| 落地：每个有落地的会话一条 devlog（触发 / 格式 / 落哪条提交） | `.agents/docs/AGENTS.md` R7 | Gates → Before landing | ✅（`c186cf5`） |
| 落地：三个文档检查脚本必须 exit 0 | `.agents/AGENTS.md` 脚本一节 | Gates → Before landing（本次补） | ✅ |
| 落地：计划行 / 审计条回填 | `hok-worktree-parallel` §收尾四件套（清单本体）+ `pending-tasks` 与各审计文档（回填落点） | Gates → Before landing（指向四件套本体） | ✅ |
| 收尾：报告归档 / 双轴归属 / 索引更新 | `.agents/docs/AGENTS.md` R1–R9 | Gates → Before landing + Agent workspace | ✅ |
| 全程：token 最省（不整拉文件、不回灌 stdout） | 全局 `token-efficient-processing` | Always-on（本次补） | ✅ |
| 全程：写文件 UTF-8 无 BOM + LF + 末行换行 | 全局 `file-editing` | Always-on（本次补） | ✅ |
| 全程：交互语言跟随用户、产物跟项目 | 全局 `chinese-interaction` + `.agents/AGENTS.md` 语言约定 | Always-on（本次补） | ✅ |
| 全程：禁补丁式修复 | 全局 `no-patchwork-fixes` | Always-on（本次补） | ✅ |
| 会话跑偏：强提醒另开对话 | 全局 `session-scope-guard` | Session hygiene | ✅ |
| 开 worktree：base 取主线当前 tip / 独立 target / 只信自建副本 / 禁 merge commit | 全局 `worktree-parallel` + `hok-worktree-parallel` | Parallel work → Worktrees | ✅ |
| 新 worktree：先 `cargo build -p hok-shim`，否则 `crates/libscoop/build.rs:204` panic | `hok-worktree-parallel` | Parallel work → Worktrees（本次补） | ✅ |
| 派活前：文件+语义重叠矩阵、并发 2–3、prompt 契约 | 全局 `agent-orchestration` + `hok-agent-orchestration` | Parallel work → Orchestration | ✅ |
| 暂存：按主题显式列文件、禁 `git add -A` / `.`、禁 `git clean -fd` | 全局 `git-commit-practice` + `.agents/AGENTS.md` | Gates → Before staging + Parallel work | ✅ |
| 改码前读专题技能（nsis / time / junction / shim / jsonpath / git pull / …） | 各技能自身 | 无 | **接受**：这是"知识"，不是"闸门"，靠 `description` 命中即可 |

## 2. 本次已补的锚点

§1 标"本次补"的 6 处，外加：`.agents/AGENTS.md` 新增「规则分层」一节（判据本身要有归属）、
`.agents/skills/AGENTS.md` 删掉过期行数（规则文件不追记会漂的数字）、本文件登记进 `.agents/docs/README.md`。

## 3. 已处置（复述收敛 + 根 `AGENTS.md` 两处措辞）

> 本节原为待办（当时写作「8 处」，逐组落点实为 **7 组**）。2026-09-26 全部落地：每组只留**本体**，
> 其余降级为**指针**。提交：`9eea4a6`（worktree / 落地族）、`c2fa597`（去重门槛 + description），
> 以及本次回填所属的收尾提交（根措辞 + 本节）。

1. **复述收敛（7 组）** —— 最终本体与其余落点：
   - **热点文件清单**（5 处：`.agents/AGENTS.md` 硬约束 1 与「提交约定」5、根 Parallel work、
     `hok-worktree-parallel`、`hok-agent-orchestration`）→ 本体 = `.agents/AGENTS.md` 硬约束 1（唯一带理由）；
     `hok-worktree-parallel` 保留本域特有的「子代理禁止写、回填由父会话做」。
   - **主线名来源**（3 处：根 Parallel work、`pending-tasks.md` §0 基线、`hok-worktree-parallel`）
     → 本体 = `hok-worktree-parallel`「本仓事实」（本机值 + 重设命令）；根不再把主线写成 `main`。
   - **收尾四件套**（根 Before landing、`.agents/AGENTS.md`「提交约定」2、`hok-agent-orchestration` §7、
     `hok-worktree-parallel`）→ 本体 = `hok-worktree-parallel` §收尾四件套（本域内容是「回填哪一行」）。
   - **去重量化门槛**（全局 `proactive-review` / `hok-code-organization` / `hok-proactive-review`）
     → **推翻本节原建议**：原判「保留 `hok-code-organization`」，但那会让全局技能反向依赖某个仓库的技能。
     改走「通用门槛留在全局」——本体 = 全局 `proactive-review` §7；`hok-code-organization` 只留 P2-6 判例数字
     与「解决什么真实问题」，`hok-proactive-review` 只留指针，根 Before implementing 只点名两者（不写参数）。
   - **`hok-report-archive` 复刻 R1 双轴表** → 删表，只留「开工扫哪里、收尾改哪些索引」+ 一句指向 R1。
     原表 12 行逐行核对后，唯一**不是** R1 复述的一行（仓库根 `AGENTS.md` / `ARCHITECTURE.md` / `README*.md` /
     `CHANGELOG.md` 不参与归档改名；根 `docs/` 的归属）改写成「射程」一段保留（后续 `docs(agents)` 小提交补回）。
    教训：**删复述表前先逐行核对"这行在别处是否还有本体"**——边界行（本技能不碰什么）常混在复述行里。
   - **语言约定表**（根 `> Language convention` 与 `.agents/AGENTS.md` 语言约定）→ 本体 = `.agents/AGENTS.md`；
     根只留一句指向。
   - **技能 `description` 复述规则**（`hok-worktree-parallel`、`scoop-gap-audit`）
     → 只写「谁在什么场景加载」：保留 `P0–P3` 之类发现关键词，去掉正文规则摘要（description 是唯一发现入口，
     不剥到加载不出场景）。
2. **根 `AGENTS.md` 命令计数** → 改指 `src/cmd/mod.rs` 的 `Command` 枚举（实测 **37** 个变体，与根的命令清单一致），
   并补上「加命令还要往该枚举加变体」这一步；不再指向 `src/cmd/` 目录（该目录 39 个条目，含生成物）。
3. **根 `AGENTS.md` 主线名措辞** → 删掉写死的 `` `main` `` 与配置键名，改为「名字来自机器本地 git 配置，
   取值与重设见 `hok-worktree-parallel`」——根治的是**并列读法**，不是值本身。

**刻意不动的**（复述判据的边界，供下次参照）：

- `pending-tasks.md` §8.5 收口检查单 / §8.8 开工卡片、`hok-agent-orchestration` §2 的 prompt 契约：
  它们是**自包含**的派活与检查清单；其中的 `git rev-parse --short $(git config …)` 是**使用**，
  不是**规则陈述**（后者才是要收敛的复述）。
- `pending-tasks.md` §8.4/§8.7 与历史文档里的 tip 数字：快照，已被正文标注为「别当待办」。
- 冻结区（`reports/**`、`devlog/**`、`archive/**`）里的同名文字：历史原文，R5 禁止"修正"。

## 4. 已否决 / 不改（附理由）

- **`edition 2024` 字面量**（根 Deterministic wrap-up、`hok-rust-gate-practice`）：全局 `rust-deterministic-gate`
  禁的是把 edition 写进 **gate 命令**（`cargo fmt` 不写 edition）；根里那句是仓情陈述而非命令参数。
- **`CLAUDE.md` 不在 `doc-links.mjs` 覆盖范围**：该文件仅 1 行 `@AGENTS.md`，无链接可断。
- **`worktree-merge.mainbranch` 与全局脚本 `mainBranch` 的大小写差**：git 键名大小写不敏感、实测同值，
  属措辞而非行为差异。

## 5. 复核记录（父会话对"要写进常驻层"的断言逐条自查）

- ✅ `crates/libscoop/build.rs:204-207`：`panic!("hok-shim binary not found at …\n  Run \`cargo build -p hok-shim\` first.")`，与审计一致。
- ✅ 根 `AGENTS.md` 全文 grep：`autocrlf` / `eol` / `commit -F` / `devlog-index` / `doc-links` /
  `docs-index-check` / `token-efficient` / `file-editing` / `chinese-interaction` / `no-patchwork` /
  `hok-code-organization` 全部 0 命中——"缺锚点"成立。
- ✅ `.agents/scripts/`：`git ls-files` 实测 11 个非 `.mjs` 跟踪文件，登记确实不全。
- ❌ 子代理给出的技能体积数据有误（称 `scoop-gap-audit.md` 为 4225 B，实测 **6510 B / 51 行**）→ 结论不受影响，
  但**行数/体积类数字不写进规则文件**（`skills/AGENTS.md` 的过期数字已同批删掉）。
- ⏸ 本轮未核实：全局技能 `file-editing` 正文。（`~/.dsh/skills/.archive/` 已不存在——全局技能根已整体迁至
  `~/.agents/skills`，归档在同级 `~/.dsh/skills-archive/`。）

## 6. 本轮已处置的缺口（跟进记录）

- **`.agents/scripts/` 与 R8 的冲突**（原则是"用完即删"，实验室在 `.agents/tmp/`）。按"是否还要再跑一次"逐项处置：
  - **删**：`json-tolerance-probe.ps1`、`private-bucket-probe.ps1`——两者在文档里只以 `.agents/tmp/` 路径被引用
    （`active/audit/optimization-audit-2026-09.md`、`active/plan/private-bucket-plan-2026-09.md`），跟踪副本是
    入库时的遗留，**可从 `b8ce863` 取回**。
  - **留并登记**：`cleanup-bench/`（`docs/research/cleanup-delete-perf.md` 标为"复跑"、`active/spec/cleanup-parallel-spec.md`
    标为"捕获工具链"）、`help-audit.js` 与 `dump-help.js`（`reports/prompts/clap-arg-parsing-corruption-prompt.md`
    给出调用命令；该报告已冻结，删脚本会让复现路径失效）。
  - 连带：`.agents/AGENTS.md` 的「脚本」一节把三类分开列（门禁 / 可复跑调查工具 / 用完即删），规则句补上
    "实验室是 `.agents/tmp/`"。
- **全局技能根边界：决定不搬，改为明写**（原为"搬进 `.agents/skills/` 还是写清边界"的决策门）。
  - 理由：本目录与机器本地根**同名即覆盖**（实测 `.dsh/skills` rank 100 压过 `.agents/skills` rank 200，
    日志报 `ignored because a higher-priority skill already exists`）⇒ 把 `investigate-first` 等原名搬进来
    会被全局那份静默压掉；改名成 `hok-investigate-first` 则同时存在两份定义，正好是我在 §3 要收敛的"复述漂移"。
  - 处置：根 `AGENTS.md`（Agent workspace）点明这些闸门由机器本地技能承载、**缺该根则闸门不生效**；
    `.agents/skills/AGENTS.md` 新增「与全局技能根的分工」，写明不得同名、需要补充时用 `hok-*` 且只指向本体。
