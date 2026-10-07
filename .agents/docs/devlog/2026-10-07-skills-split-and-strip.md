# 全局技能拆分 + hok 技能去史

> 2026-10-07 ｜ 提交 `ec9c139` `1341dd2` ｜ worktree 主树（`D:\App\github项目\Scoop\hok`）｜ 分支 `refactor/edition-2024`

## 背景

本机 `~/.agents/skills/` 的 `agent-workspace-conventions` 与 `worktree-parallel` 双双贴线：12,136 B
（自查线 12,288 B，只剩 152 B 余量）与 15,220 B（已超）。用户要求拆分减重，随后要求顺带复核 hok 自己的
16 个项目技能。两件事同批做完。

## 改了什么

**全局（机器本地，不入本仓）**：`agent-workspace-conventions` 的「技能规范」章 → 新技能 `skill-authoring`
（`templates/skill.md` 随本体迁移）；`worktree-parallel` 的「编排」章 → 新技能 `agent-orchestration`，
「配套脚本」章 → 技能包内 `scripts/README.md`。两份 SKILL.md 落到 9.3 KB / 6.7 KB，新技能 4.0 KB / 7.5 KB。

**本仓 `ec9c139`（2 文件）**：根 `AGENTS.md` 的 Orchestration 闸门、`rule-gate-anchors-2026-09.md` 的
本体登记，一并改指 `agent-orchestration`——拆分打断了 `worktree-parallel §9` 这个引用。

**本仓 `1341dd2`（11 文件）**：按 `.agents/skills/AGENTS.md` 的标准逐个复核 16 个技能：

- `scoop-gap-audit` 的 manifest 进展日志（P0-1/P0-2/P0-4、两条已解决的"待办"、假 flaky 闩锁）
  **移入** `active/audit/scoop-gap-status.md` 新增的「manifest 解析专题」；原处只留方法。
- `hok-nsis-extraction` 重写：已解决项（solid 预算、传统 NSIS 目录结构、zstd 非问题）是记录不是规则；
  保留架构、开放局限、反模式清单。
- `hok-agent-orchestration`：删掉复述全局 `agent-orchestration` 的整章与会话代价叙事，只留 hok 落点。
- `hok-worktree-parallel`：删提交 sha、会话号、迁移叙事、已废的四件套条目。
- 其余六个技能去状态化、去掉计划项编号、把"事后补 fix/测试"改成它本该触发的检查。
- 修 `../Scoop/...`：从 `.agents/skills/` 出发解析到不存在的目录；4 处改指 `local-scoop-source-path`。

## 教训

1. **技能会以"又踩了一个坑"的方式只增不减**——自查线放在技能规范里，但真正触发的往往是它自己。
2. **拆技能前先 grep 入站引用**：`§9`/`§8` 这类节号引用会被拆分打断，本仓有 4 个文件受影响。
3. **"删"之前先确认新家**：本次差点直接删掉技能里的进展日志，而它们并不在状态表里——先搬再删。
4. **相对路径按引用点解析**：`../Scoop` 写在仓库根的对、写在 `.agents/skills/` 的错。
5. **删计数类内容会真的丢信息**：「8 条子代理并行的代价」只存在于技能里，按标准（计数不进技能）删掉，
   其结论已作为规则留在技能中。

门禁：`devlog-index.mjs --check` / `doc-links.mjs` / `docs-index-check.mjs` 全部 exit 0。
