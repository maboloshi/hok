# 规则载体的闸门锚点审计（2026-09）

Status: active
Updated: 2026-09-26

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
| 重构/去重前：先问"解决什么真实问题" + 四项量化门槛 | `hok-code-organization` | Gates → Before implementing（本次补） | ✅ |
| 提交前：整体 review（行为锚点/边界/架构/收益） | 全局 `proactive-review` + `hok-proactive-review` | Gates → Before committing | ✅ |
| 暂存前：一主题一 commit / `-c core.autocrlf=false` / `git commit -F <file>` | 全局 `git-commit-practice` | Gates → Before staging（本次补） | ✅ |
| 收尾：fmt / clippy / test 确定性闭环 | 全局 `rust-deterministic-gate` + `hok-rust-gate-practice` | Gates → Deterministic wrap-up | ✅ |
| 落地：每个有落地的会话一条 devlog（触发 / 格式 / 落哪条提交） | `.agents/docs/AGENTS.md` R7 | Gates → Before landing | ✅（`c186cf5`） |
| 落地：三个文档检查脚本必须 exit 0 | `.agents/AGENTS.md` 脚本一节 | Gates → Before landing（本次补） | ✅ |
| 落地：计划行 / 审计条回填 | `pending-tasks` 与各审计文档 | Gates → Before landing（指向四件套本体） | ✅ |
| 收尾：报告归档 / 双轴归属 / 索引更新 | `.agents/docs/AGENTS.md` R1–R9 | Gates → Before landing + Agent workspace | ✅ |
| 全程：token 最省（不整拉文件、不回灌 stdout） | 全局 `token-efficient-processing` | Always-on（本次补） | ✅ |
| 全程：写文件 UTF-8 无 BOM + LF + 末行换行 | 全局 `file-editing` | Always-on（本次补） | ✅ |
| 全程：交互语言跟随用户、产物跟项目 | 全局 `chinese-interaction` + `.agents/AGENTS.md` 语言约定 | Always-on（本次补） | ✅ |
| 全程：禁补丁式修复 | 全局 `no-patchwork-fixes` | Always-on（本次补） | ✅ |
| 会话跑偏：强提醒另开对话 | 全局 `session-scope-guard` | Session hygiene | ✅ |
| 开 worktree：base 取主线当前 tip / 独立 target / 只信自建副本 / 禁 merge commit | 全局 `worktree-parallel` + `hok-worktree-parallel` | Parallel work → Worktrees | ✅ |
| 新 worktree：先 `cargo build -p hok-shim`，否则 `crates/libscoop/build.rs:204` panic | `hok-worktree-parallel` | Parallel work → Worktrees（本次补） | ✅ |
| 派活前：文件+语义重叠矩阵、并发 2–3、prompt 契约 | 全局 `worktree-parallel` + `hok-agent-orchestration` | Parallel work → Orchestration | ✅ |
| 暂存：按主题显式列文件、禁 `git add -A` / `.`、禁 `git clean -fd` | 全局 `git-commit-practice` + `.agents/AGENTS.md` | Gates → Before staging + Parallel work | ✅ |
| 改码前读专题技能（nsis / time / junction / shim / jsonpath / git pull / …） | 各技能自身 | 无 | **接受**：这是"知识"，不是"闸门"，靠 `description` 命中即可 |

## 2. 本次已补的锚点

§1 标"本次补"的 6 处，外加：`.agents/AGENTS.md` 新增「规则分层」一节（判据本身要有归属）、
`.agents/skills/AGENTS.md` 删掉过期行数（规则文件不追记会漂的数字）、本文件登记进 `.agents/docs/README.md`。

## 3. 待办（未落地，各自独立主题）

1. **复述收敛（8 处）**——同一规则除本体外应只留引用：
   - 热点文件清单 5 份（`.agents/AGENTS.md` 硬约束 1、其「提交约定」节、根 Parallel work、
     `hok-worktree-parallel`、`hok-agent-orchestration`）→ 保留 `.agents/AGENTS.md` 那份（唯一带理由），
     其余改指针；`hok-worktree-parallel` 里"子代理禁止写、回填由父会话做"是本域特有内容，保留。
   - 主线名来源 3 处 → 保留 `hok-worktree-parallel`（带本机值与重设命令）。
   - 收尾四件套 4 处 → 保留 `hok-worktree-parallel` §收尾四件套（本域内容是"回填哪一行"）。
   - 去重四项门槛 3 处（全局 `proactive-review` / `hok-code-organization` / `hok-proactive-review`）
     → 保留 `hok-code-organization`（带 P2-6 案例）。
   - `hok-report-archive` 复刻 R1 双轴表 → 降级为指针，只留"开工扫哪些目录 / 收尾改哪些索引"。
   - 语言约定表（根 §Conventions 与 `.agents/AGENTS.md` 语言约定）双份 → 根只留一句指向。
   - 技能 `description` 复述规则（`hok-worktree-parallel`、`scoop-gap-audit`）→ description 只写"何时加载"。
2. **`.agents/scripts/` 登记不全**：`git ls-files` 实测 11 个跟踪的非 `.mjs` 文件（`dump-help.js`、
   `help-audit.js`、两个 `*.ps1`、`cleanup-bench/` 7 个含 `.bin` 样本），`.agents/AGENTS.md` 只登记 3 个
   `.mjs`，与 R8「一次性脚本用完即删」冲突 → 决定"删"还是"登记为调查实验区"。
3. **根 `AGENTS.md` 命令计数措辞**（"the count lives in `src/cmd/`"）：该目录实际 39 个条目（含生成物
   `__cmd_reg__.rs` 与 `shared_args.rs`）→ 建议改指 `src/cmd/mod.rs` 的 `Command` 枚举。
4. **主线名措辞**：根 Parallel work 写死 `` `main` ``，本机 `git config worktree-merge.mainbranch` =
   `refactor/edition-2024`；虽有一句"以 config 为准"，两处并列仍易误读。

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
- ⏸ 本轮未核实：全局技能 `file-editing` 正文、`~/.dsh/skills/.archive/`。
