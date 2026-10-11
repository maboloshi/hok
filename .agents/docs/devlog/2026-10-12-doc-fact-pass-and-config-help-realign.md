# 0.3 事实复核：DESIGN 拆分、config 帮助手册对齐与发布提交重做

> 2026-10-12 ｜ 提交 `386975b`（并入 0.3.0 发布提交；重做的旧发布提交保留在本地 tag `backup/pre-rewrite-v030-8096c11`）｜ worktree 主树 ｜ 分支 `main`

本会话对 0.3.0 做了一遍端到端事实复核，并把「设计与工程取舍」从两份 README 里拆成
`DESIGN.md` / `DESIGN-zh.md`。三个只读子代理分别审 CLI 表面、从 `.agents/docs/**` 挖 0.3 事实、
逐条核五份根文档；父会话用 `hok.exe`（新构建）做决定性验证。

## 1. config：文档表面与代码脱节

`hok config --help` 的设置手册（`locales/*.yml` 的 `config.long_about`）与代码不一致，而 README 又让用户
拿它当"完整参考"：`aria2-min-split-size` 写默认 `5M`（实际 `20M`，`config.rs:600`；20M vs 官方 5M 属
`pending-tasks` 已登记暂缓项）、漏 `use_git_history` 与 `autostash_on_conflict`、把可写的 `last_update`
说成只能改文件。`config -a` 里 `use_git_history` 的描述列为空——`description()` 少一条 arm，而
`every_supported_key_has_a_description` 的键表也漏了这个键，所以守卫一直没红。

修：两份 locale 的手册、`description()` 的 arm 与守卫键表；两份 README 的设置表补齐为 27 键并对齐 `-a`。

## 2. DESIGN 拆分与文档归属

README 首屏被工程细节压住 ⇒ 新建 `DESIGN.md` / `DESIGN-zh.md`（为什么这么做：依赖策略、两种 SQLite
形状、异步取舍、shim/i18n 设计、护栏与基准），README 只留使用路径；README 里与 `ARCHITECTURE.md`
重复的 Architecture 段**删除并留指针**（避免第二处本体）。归属本体仍是 `ARCHITECTURE.md` 的 Doc Map，
根 `AGENTS.md` 加了一行指针；顺带修掉 `docs/README.md` 里"`.agents/` 未入库"的失效陈述。

## 3. AGENTS.md 补 0.3 硬约束

新增 "Silent-breakage guards"：写盘命令必须给 `-d`、确认门三层（`--confirm` / `-y, --assume-yes` /
配置 / 非 TTY 不拦、拒绝=exit 0）、0.6.0 元数据文件名、共享 `{SCOOP}/scoop.db` 与 `@version` 历史表、
PS 解压 helper 只写 `[[HOK_EXTRACT]]` 标记、新增 `unsafe` 必须登记；补 NSIS 语料测试 `#[ignore]` 的
覆盖盲区，并把 CI 实为 `disabled_manually`、`dev`/`main` 分工写实。

## 4. 有意例外：重写已落地的发布提交

远程 tag `v0.3.0` 原指 `70da8a2`（发布提交重做前的孤儿版本），`main` 与本地 tag 已是 `8096c11`，
GitHub Release 的 `hok.exe` 也来自那棵孤儿树。按用户明确指示把本轮改动并入发布提交
（`8096c11` → `386975b`）、tag 重打并强推——这是对「不重写已落地提交」规则的一次**有意例外**，
起因是 0.3.0 发布当天自己就需要重做；旧对象由本地 `backup/pre-rewrite-v030-8096c11` 保底。

## 5. 门禁

`cargo test --workspace` / `clippy --workspace --all-targets` / `rustfmt --check`（改动文件级）通过；
`devlog-index.mjs --check`、`doc-links.mjs`、`docs-index-check.mjs` 均 exit 0。
发布附件需重跑 `release.yml`：`locales/**` 改了，旧 `hok.exe` 与 tag 已不同树。
