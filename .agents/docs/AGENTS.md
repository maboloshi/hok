# AGENTS.md — `.agents/docs/` 文档规范

本文件是 `.agents/docs/` 的**规则本体**。DSH 会在文件工具触碰本目录下文件时自动加载它，
所以下面的规则常驻生效，不依赖 `[[hok-report-archive]]` 技能被加载。

## R1 双轴归属

每篇文档同时有两个轴：**生命周期**（放哪、是否归档）与**类型**（子目录）。

| 生命周期 | 目录 | 含义 |
|---|---|---|
| 活 | `active/` | 仍在指导当前工作 |
| 活 | `research/` | 调研结论已定，不再驱动计划（但未被取代，故不归档） |
| 活 | `devlog/` | 开发日志：**一条一文件** |
| 死 | `reports/` | 已被取代 / 已闭环 → **永久冻结** |
| 死 | `legacy/` | 失效提案、上游 issue/PR 草稿与补丁 |

| 类型 | `active/` 下子目录 | 判定 |
|---|---|---|
| 规格 | `active/spec/` | **会被修订**：设计、行为规格、参数契约 |
| 计划 | `active/plan/` | **会被执行掉**：有决策门 / 验收标准 / 提交拆分 |
| 审计 | `active/audit/` | **随修复更新**：对标差距、违规清单、性能基线 |
| 调研 | `research/` | 结论一次性给出，之后只读 |
| 日志 | `devlog/` | 见 R7 |
| 会话输入 | `reports/prompts/` | 交给子代理/会话的 prompt 原文，非文档 |

判定口诀：**会变的进 `active/`（按怎么变选类型），不会再变的进 `research/` 或 `reports/`。**

## R2 命名

全小写 kebab-case；**日期只在"名不副实"时才有意义**，按文档性质分档：

| 文档性质 | 形式 | 例 |
|---|---|---|
| 活文档（持续演进） | `<主题>.md`，**不带日期** | `active/spec/output-system-design.md`、`active/audit/unsafe-audit.md`、`active/plan/pending-tasks.md` |
| 一次性快照（审计 / 调研） | `<主题>-<YYYY-MM>.md`，日期到**月** | `active/audit/command-deviation-audit-2026-09.md`、`research/cleanup-delete-perf.md` |
| 日志条目 | `<YYYY-MM-DD>-<english-slug>.md`，完整日期 | `devlog/2026-09-26-manifest-cache-warn-latch.md` |
| 索引 / 规范 | 固定名 | `README.md`、`AGENTS.md` |

- **slug 一律英文 kebab**（日志条目同）；中文只出现在文件**内**的 `# ` 标题行。
- **禁冗余后缀**：`-research`（已在 `research/` 内）、`-audit` / `-investigation`（已在 `audit/` 内）。
- **禁**无主题名（`analysis-report`）、全大写（`FINAL_AUDIT.md`）。
- **规则只管新建**；历史文件不强制改名。唯一例外——**名字会说谎就立即改**：
  2026-09-26 把 `active/plan/pending-tasks-2026-09.md` 改为 `pending-tasks.md`
  （开放式待办总表，月份后缀注定过期）。
- `reports/**` 是冻结区，**永不改名**（R5）；从 `reports/` 提回 `active/` 时，若名不符规则就在同一次改动里改。

## R3 头块

`active/**` 与 `research/**` 的新文档，前 5 行固定：

```markdown
# <标题>

Status: active | closed | superseded-by <相对链接>
Updated: YYYY-MM-DD
```

- 归档时在 `Status:` 下一行插入 `Archived: YYYY-MM-DD`（见 R5）。
- `reports/` 中历史冻结文档**不回填**。

## R4 归档判定

满足其一即归档：

1. 被新报告明确取代；
2. 结论已吸收进技能记忆（`.agents/skills/`）；
3. 会话性总结已完成使命；
4. **计划类：全部条目已落地，或已转入新计划** —— 归档时保留一行「未落地项去向」。

`active/` 与 `reports/` 之间的移动是**双向**的：`reports/README.md` 状态列写着"当前有效"的，
说明归档判断当时就错了，应提回 `active/`。2026-09-26 已按此提回 5 篇。

## R5 归档动作（自由化）

1. `Move-Item` 移入 `reports/`（或 `legacy/`）；
2. **正文一字不动**，只插入 `Archived: YYYY-MM-DD`（`Status:` 下一行）；
3. 更新 `reports/README.md` 索引（R6）；
4. 修 inbound link（`grep -r "<旧文件名>" .agents AGENTS.md README*.md`）。
5. **整目录搬迁的例外**：若归档的是整个目录（如 `active/plan/refactor/` → `reports/refactor/`），
   允许在**同一次改动**里把该目录**内部**的路径前缀统一改掉——只改路径，不改正文。
   不这么做的话，归档件之间会立刻互相断链（DSH 的做法是原样冻结并由门禁跳过归档源，本仓库没有门禁）。

归档后**永久冻结**：不得编辑、翻译、重排、再移动。历史快照不是当前行为的权威。

## R6 索引（与 DSH 相反，故意保留中央索引）

- `docs/README.md` —— 活文档总索引；
- `docs/reports/README.md` —— 归档清单（含"归档原因"列）；
- `docs/devlog/README.md` —— **脚本生成，禁手改**。

**新文档必须在同一次改动里进索引。** 2026-09-26 发现 `reports/README.md` 漏登记 4 篇
（`h3-parse-visibility-design`、`manifest-runtime-tolerance-scoping`、`rars-winsqlite3-handover`、`status-gap-analysis`），
就是"先落文件、忘了登索引"的后果。

> DSH 官方仓库明令禁止中央索引（`.agents/notes/README.md`），因为它有 `verify-agent-note-format` 之类的门禁脚本可替代。
> 本仓库没有门禁，人工索引成本低、收益实，故**保留**。

## R7 日志：一条一文件

- 位置：`docs/devlog/YYYY-MM-DD-<english-slug>.md`（**扁平**，按文件名排序即时间序）。
  索引 `docs/devlog/README.md` 由 `scripts/devlog-index.mjs` 生成，**禁手改**。
- 条目格式（脚本按此解析）：

  ```markdown
  # <主题>

  > <YYYY-MM-DD> ｜ 提交 `<sha>`[, `<sha>`…] ｜ worktree `<path>` ｜ 分支 `<name>`

  <正文；子章节用 ## >
  ```

  索引脚本：日期取自**文件名**，主题取自首行 `# `，提交 sha 取自第 3 行的反引号内容。
- slug 用英文 kebab（`manifest-cache-warn-latch`、`7z-empty-entries`）；中文标题留在文件内的 `# ` 行。
- **单条正文控制在 40 行以内**；超过就说明它该是一篇 `research/` 或 `reports/` 文档，日志里只留结论 + 链接。
- 日志不归档、不进 `docs/README.md`（索引本身已按日期分页）。
- 历史：2026-09-26 由 137 KB 单体 `devlog.md` → 月分片 → **34 个一条一文件**（自 `2026-08-15-nsis-extraction-and-7z-perf.md` 起）。

## R8 脚本

- 可复用脚本一律 `.agents/scripts/`；**不得**再出现 `docs/scripts/`。
- 一次性脚本用完即删，禁止散落在 `docs/` 或项目根。

## R9 引用规则

`.agents/docs/**` **已入库**（2026-09-26 起）⇒ 从任何入库文件引用它都是合法的。

- 引用**必须用仓库相对路径**（如 `.agents/docs/active/audit/unsafe-audit.md`），不得写本机绝对路径。
- 历史记录（`reports/`、`devlog/`、`legacy/`、`research/`）里保留作者本机的绝对路径**属正常**——
  那是当时的原始记录，不要"修正"。
- 面向**仓库外**读者、需长期维护的文档仍放仓库根 `docs/`；`.agents/docs/` 是维护者内部语料。
