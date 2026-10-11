# pending-tasks 逐条复核：关闭已落地项，开放项补今日证据

> 2026-10-05 ｜ 提交 `5f73d5f` ｜ worktree `.` ｜ 分支 `refactor/edition-2024`

用户要求「全部梳理一遍，把已落地的关闭」。逐行核对 492 行总表与当前树（代码 grep + 提交核对）后，
结论是**表大体已自洽**——多数落地项早就带 ✅——真正「落地了却仍写成待办」的只有 5 处：

| 关闭项 | 落地证据 |
|---|---|
| `checkver.rs` 陈旧 TODO 清理 | `b339afc`（随 S1，§6 表格早有该行） |
| §8.7 的 S1 会话行（仍写"开工实现"） | `43795484`：5 提交线性 `--ff-only`、无 merge commit |
| §8.3 Wave 1 行 | S1 `43795484` / S2 `9b672207` |
| `scripts/*.py` 与 `.agents/tmp` 重叠 | 重叠源 `analyze_cohesion.py`/`analyze_rust_project.py` 已于 2026-09-26 删除；现存 3 个脚本各有专责 |
| `reports/README.md` 索引措辞 | `optimization-audit-2026-09.md` 已按 R4 提回 `active/audit/`；`docs-index-check` 0 缺口 |

**仍然开放（复核后确认，不是漏关）**：H3（设计报告已出、D1–D5 未拍板、代码零改动）、H4
（`auto_pr.rs:22` 仍 `default_value = "bucket"`，未接 `write_dir`/`scan_dir`）、R5（全仓
`SyncOption::Cascade` 读 2 写 0）、R6（`network.rs:235-236` 仍 `.header()` append）、CheckUrls
Referer（`referer()` 对 `CheckUrls` 仍返回 `Some`）、framework P1-5（`config.rs:1234/1432` 仍在 lib 层）、
§5 私有 bucket SSH（凭据回调无 SSH 分支）、§9 X2–X4。这些行都补了「2026-10-05 复核」与当前行号。

**一处纠正**：`reports/README.md` 把 H3 设计报告的归档原因写成「已落地」——报告自己写着「尚未改任何生产
代码」，代码里也确实没有可见性告警（`Error parsing JSON at` 只在 `missing_checkver.rs:113`，是 S11 落的）。
改为「设计调研已闭环、代码未落地」，与 pending-tasks H3 对齐。

**教训**：勾状标记不会自己出现。落地方是子代理/worktree，回填方是父会话；只要回填只做"新增一行"而不
复核"旧行的状态列"，表里就会同时存在 ✅ H1 与仍写"开工实现"的 S1。**收尾回填要按行复核，不只追加**。
