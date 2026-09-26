# AGENTS.md — `.agents/` 工作区

本目录是 hok 仓库的 **agent 工作区**：存放 AI 协作产物（技能、决策记录、工作文档、脚本），
**已纳入版本控制**（2026-09-26 起）。`.gitignore` 只挡机器本地与临时内容：

```gitignore
.agents/tmp/
.agents/**/*.local.md
.agents/**/*.local.*
```

> 历史：2026-09-26 由 `.dsh/` 迁入。**禁止**再创建 `.dsh`（目录或链接）——DSH 的项目级技能根
> `.dsh/skills`(rank 100) 优先级高于 `.agents/skills`(rank 200)，两者并存会让 `.agents/skills` 被压制，
> 每个同名技能都会触发 `skill "<name>" from project-dsh ignored because a higher-priority skill already exists`。
> 证据：`@deepseek-ai/dsh-skill-filesystem/lib/index.js:21-22,150-165`、`@deepseek-ai/dsh-skill/lib/index.js:320`。

## 语言约定（2026-09-26 固定）

**按位置分档，不按内容分档：**

| 位置 | 语言 | 说明 |
|---|---|---|
| 仓库根文档（含 `docs/`） | **英文** | `AGENTS.md`、`ARCHITECTURE.md`、`README.md`、`CHANGELOG.md`、`docs/**`；中文版另开文件（现有 `README-zh.md`、`docs/reference/*.zh.md`） |
| `.agents/**` | **中文** | 技能、工作文档、各目录 `AGENTS.md` —— 本工作区是维护者的中文语料 |

- 引用路径、代码、命令、标识符一律保持原文，不翻译。
- 跨语言引用允许且常见（英文根文档 → 中文 `.agents/**` 规则），路径用仓库相对路径即可。
- 新增根文档默认英文；确实需要中文读者版时**另开 `<name>-zh.md`**，不要在同一个文件里混。

## 目录职责

| 目录 | 内容 | 规则 |
|---|---|---|
| `skills/` | 项目技能：扁平 `<name>.md`，frontmatter `name` 必须等于文件名 | [`skills/AGENTS.md`](skills/AGENTS.md) |
| `archive/skills/` | 过期技能（冻结；2026-09-26 迁移时统一修过 `.dsh`→`.agents` 路径，**按约定保留改后内容**，勿再翻改。曾与现役同名的 3 份旧副本已删——别再按同名去找） | **必须**放在技能发现根之外，理由见下 |
| `docs/` | 工作文档：规格 / 计划 / 审计 / 调研 / 日志 / 归档 | [`docs/AGENTS.md`](docs/AGENTS.md) |
| `scripts/` | 可复用脚本（索引生成、批量分析、探针） | 见「脚本」一节 |

**为什么 `archive/` 不在 `skills/` 内**：DSH 会把技能根下的一级目录当作技能包（找该目录的 `SKILL.md`），
放在 `skills/.archive/` 里虽然会被静默跳过（`parseSkillFile` 对不存在的 `SKILL.md` 返回 undefined），
但会污染"看起来是技能"的目录层级，也让 watcher 白跑。归档一律移出发现根。

## 规则分层（加规则 / 改规则前读）

DSH 只保证**常驻**文件在需要时已被注入：仓库根 `AGENTS.md` 每会话每轮都在上下文里；本文件只在文件工具
触碰 `.agents/**` 时注入；`docs/AGENTS.md`、`skills/AGENTS.md` 同理按目录；技能正文（`.agents/skills/*.md`）
只在被显式加载时进上下文，常驻的只有它的 `description`。因此：

- **闸门**（"到某个时刻必须做某动作"）**必须在根 `AGENTS.md` 有锚点**（一行：时机 → 动作 → 本体链接），
  否则到那一刻根本读不到。锚点只写时机与指向，**不写参数**。
- **知识**（"改这个子系统前要知道什么"）留在技能里：正文可写细节，`description` 只写"谁在什么场景加载"。
- **复述必漂**：同一条规则除本体外只允许出现**引用**（"见 X"）。发现第二处复述 → 收敛到本体、别处降级为指针。
  判例：日志触发曾在 8 个文件里各写一遍，措辞漂成"每条会话 / 每项修完 / 新增条目"三种。
- **落地记录 ≠ 规则**：改了什么过程写 `docs/devlog/`（R7），**不要**追加进技能正文（判据见
  [`skills/AGENTS.md`](skills/AGENTS.md)）。

现状与缺口清单：`.agents/docs/active/audit/rule-gate-anchors-2026-09.md`。

## 硬约束

1. **每个 worktree 都有一份**：本目录已跟踪 ⇒ 正常改、正常提交，rebase 冲突按普通文档解。
   **例外（这份清单是本体，别处只留指针）**：汇总型热点文件 `docs/active/plan/pending-tasks.md`、
   `docs/active/audit/command-deviation-audit-2026-09.md`、`docs/active/audit/scoop-gap-status.md`
   **只在主树改**，否则并行会话的收尾回填会互相冲撞。
2. **禁 `git add -A` / `git add .`**：提交必须按主题显式列文件；**禁 `git clean -fd`**。细则见「提交约定」。
3. **加载时机**：DSH 在文件工具触碰本目录下文件时，把「本目录 → 项目根」链路上的 `AGENTS.md`
   注入模型（证据：`@deepseek-ai/dsh-agent-instructions/lib/index.js:515-521,578,1066`）。
   因此本文件与各子目录的 `AGENTS.md` 是**常驻规则**，不依赖某个技能被加载。
4. **临时产物不落项目**：一次性日志、构建输出、下载中间物一律用系统临时目录
   （DSH 自身的溢出文件落在 `%TEMP%\dsh-spill-*`，从不落项目）。本目录下不得新建 `tmp/` 或散落 `.log`。
5. **路径要可移植**：本目录入库 ⇒ **指令层不得出现本机绝对路径**，用仓库相对路径。
   指令层 = 本文件、`skills/**`、`docs/AGENTS.md`、`docs/README.md`、`docs/active/**`（除 `prompts/` 子树）。
   历史记录 = `docs/reports/**`、`docs/devlog/**`、`docs/legacy/**`、`docs/research/**`、`archive/**`、
   以及任何 `prompts/` 子树 —— **保留原文**，那是当时的原始记录，不要"修正"。

## 提交约定（2026-09-26 固定）

本目录**已入库** ⇒ 提交行为本身也是约定的一部分。

1. **首次入库单独一条**：`chore(agents): bring the .agents workspace under version control`。
   整个目录一次进库（文件数不写死——它刚被证明会变），混进任何主题提交都会把该主题的 diff 淹掉。
2. **此后：同主题同提交**。收尾回填（**计划 / 审计 / 技能**，清单见
   [`skills/hok-worktree-parallel.md`](skills/hok-worktree-parallel.md) → 「收尾四件套」）与代码改动
   **放同一个提交**——它们是一次会话的闭环，拆开会让文档引用尚未落地的代码。**日志条目例外：自成一条收尾提交**
   （`docs(agents): log …`），因为条目第 3 行要引用本次主题的代码提交 sha，与代码同提交无法自引
   （触发与格式见 [`docs/AGENTS.md`](docs/AGENTS.md) R7）。纯文档/技能改动同样用 `docs(agents): …` 起头。
3. **显式列文件**：禁 `git add -A` / `git add .`；允许按主题显式列到文件集，例如
   `git add .agents/docs/devlog/2026-09-26-*.md .agents/skills/hok-x.md`。
   （首次入库那一条例外：允许 `git add .agents` —— 整目录就是该提交的唯一主题。）
4. **worktree 里改了本目录，必须随该主题提交落地**，不能留在 worktree 内——删掉 worktree 就丢了
   （未跟踪时代不存在这个风险）。
5. **汇总型热点文件仍只在主树改**（**清单与理由见「硬约束」1**，别处只留指针）。
   （`skills/scoop-gap-audit.md` 2026-09-26 起只是方法论，不再每会话回填。）
6. **不进的**：`.agents/tmp/`、本机 `*.local.md`（已 ignore）、**机器生成的分析 dump**
   （`*_report.txt` / `*_drywall.txt` / `*_duplicates.txt`，见根 `.gitignore`——即便它们被 `reports/README.md`
   登记也不入库；索引 ≠ 版本控制）、一次性探针脚本（用完即删；提交前用 `git status --short` 复核，
   别让"忘了删"的东西混进来）。

## 脚本

- 可复用脚本放 `scripts/`；一次性的分析/探针脚本用完即删（实验室是 `.agents/tmp/`，跟踪的副本只留给
  "还要再跑一次"的），禁止散落在 `docs/` 或项目根。
- **门禁脚本**（收尾一次跑齐）：
  - `devlog-index.mjs` —— 生成 `docs/devlog/README.md`（`--check` 为门禁模式）；
  - `doc-links.mjs` —— 文档相对链接完整性检查（`.agents/**` + `docs/**` + 根 `*.md`，断链则 exit 1）；
  - `docs-index-check.mjs` —— 索引覆盖率检查（R6：`docs/` 下的文件是否都已登记，缺口则 exit 1）。
- **可复跑的调查工具**（被活文档引为复跑入口，故保留）：
  - `cleanup-bench/` —— `cleanup` 删除性能与 TTY 实测 harness（`bench.rs`/`bench2.rs`/`probe3.rs` +
    ConPTY 宿主 `conpty-capture.rs` + 真 TTY 录制样本；入口见 `docs/research/cleanup-delete-perf.md`
    与 `docs/active/spec/cleanup-parallel-spec.md` 的「捕获工具链」）；
  - `help-audit.js` —— 枚举全部帮助面并报告失败：`node .agents/scripts/help-audit.js <hok.exe>`；
  - `dump-help.js` —— 逐面 dump 供改动前后逐字节比对：`node .agents/scripts/dump-help.js <hok.exe> <out-dir>`。

  收尾建议一次跑齐：

  ```powershell
  node .agents/scripts/devlog-index.mjs            # 重新生成 devlog 索引
  node .agents/scripts/devlog-index.mjs --check    # 索引过期则 exit 1
  node .agents/scripts/doc-links.mjs               # 断链则 exit 1
  node .agents/scripts/docs-index-check.mjs        # 索引有缺口则 exit 1
  ```

## 已知环境坑

- 运行中的 DSH 会话会用 chokidar 持有 `<项目根>/.agents/skills` 的目录句柄，
  导致该目录**无法重命名或删除**（`EPERM`）。改结构时先搬里面的文件，目录留到会话结束后清理。
