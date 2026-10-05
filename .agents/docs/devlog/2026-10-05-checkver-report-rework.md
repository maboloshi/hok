# hok checkver 输出重做：符号结果行 + 单行转圈 + 汇总

> 2026-10-05 ｜ 提交 `cb1fd39` ｜ worktree `.worktrees/checkver-output` ｜ 分支 `checkver-output`

`hok checkver` 的报告重做成可读、可 grep、可管道的形态：符号结果行（`↑ ↓ ✓ ✗ !`，版本增量按
dim/accent/success 着色）、TTY 下一条聚合转圈行、尾部汇总；**错误从 stderr 搬到 stdout**（报告就是
这条命令的输出，重定向时以前会丢掉全部错误）；抽取失败改说上游原话 `couldn't match '<regex>' in
<url>` / `couldn't find new version in <url>`（`bin/checkver.ps1:408,414`）。新增 `-j/--jobs`
（默认 8，`0` = 官方的不限）。

## 若干判断

- **顺序按 app 模式分流**：点名包 → 清单序（队首阻塞），默认 `*` → 完成序。兄弟命令都是顺序
  `for`，「完成序 ≡ 清单序」，所以这条规则当前只在 checkver 有实际区别。
- **不做「每清单一转圈行」**：`MultiProgress` 会重绘所有未清除的行，整 bucket 就是 O(N)/tick；
  单行聚合是 O(1)/tick。这是对 `output-system-design.md` §八 B 的收窄，理由记在 §11.5。
- **`MultiProgress::new()` 画到 stderr**（indicatif 0.18.6 `src/multi.rs:144`）。报告类命令的行
  就是它的产物，必须 `ProgressDrawTarget::stdout()`（新增 `RowUI::on_stdout`），否则结果行跟着
  转圈一起进 stderr。
- **`-s/--skip-updated` 本来就是**「只输出可更新或错误的」——需求 2 没有新开关可加，只改了帮助
  文案；被它压掉的清单仍进汇总（`CheckverSummary` 在 lib 侧记账，不靠数 report 条数）。
- **汇总单位用 `manifest(s)`**：i18n 不做复数，`1 manifests` 是错的，同仓 `formatjson_count`
  已是这个写法；标签也用数量无关的分词，避免 `1 errors`。

## 验证

`cargo test --workspace` 绿、`clippy --workspace --all-targets -D warnings` 净、文件级 rustfmt
已跑。本地 HTTP 夹具（三份清单、第一份故意慢 300ms）端到端确认两种排放顺序、`-s` 压制但汇总仍
准确、stderr 为空。

**未验证**：TTY 转圈行本身。`.agents/scripts/cleanup-bench/conpty-capture.rs` 不给子进程 TTY
（实测子进程 `process.stdout.isTTY === undefined`，捕获恒为空帧），所以只覆盖了非 TTY 路径。

## 顺带发现

`doc-links.mjs` 在 worktree 里**是空转的**：`SKIP_DIR` 含 `.worktrees`，而 worktree 的路径必然带
这段，于是 `.agents/**` 整棵树被跳过（实测只检查 6 个文档，主树是 156）。worktree 里的绿灯不代表
链接没断——收尾要在主树跑。同类兄弟命令的统一是后期专项，见 `pending-tasks.md`。
