# health 补两项检查：悬空 current 链接 + 断面 shim

> 2026-10-07 ｜ 提交 `dd52a4f` ｜ worktree `.` （主线 `refactor/edition-2024`）｜ 分支 `refactor/edition-2024`

## 落地项（本次提交内容）

- `crates/libscoop/src/package/shim.rs`：`shim_target` 拆为 `shim_target_raw`（只解析声明目标）+ 存在性探测（`which` 语义不变）；新增 `BrokenShim` 与 `broken_shims(session)`。
- `crates/libscoop/src/package/health.rs`：用 `read_link` 区分"链接缺失"与"链接悬空"（新键 `health_current_dangling`）；包扫描后追加断面 shim 问题（`health_shim_broken`）；shim 目录读失败不吞错（`health_shims_unreadable`）。
- `locales/en.yml` / `locales/zh.yml`：三个新键。
- 计划回填：`.agents/docs/active/plan/cli-additions-plan.md` 第 1 项标完成，第 6 项（`service`）标暂缓。

## 两个判断依据

- **放 `health` 不放 `checkup`**：官方 `scoop-checkup.ps1:11-47` 只有 9 条**环境**判定，从不检查 shim/junction；hok `checkup` 已对齐官方，改动它就是静默偏离基线。`health` 是 hok 独有命令，语义就是"已装包的问题"。
- **断链判定取保守侧**：所有文本变体都解析失败才算断裂，宁漏报不误报（用户还能跑的 shim 不报）；`.exe` 是启动器不读，`scoop-*.ps1` 别名脚本跳过。

## 自查发现并修掉的 bug

`broken_shims` 的变体列表最初写成 `["", ".cmd", ".ps1"]`，**漏了 `.shim` 本身**——即最常见的元数据文件。测试直接把它照出来（断面 `.shim` 未被计入）。教训：变体枚举要与 `shim_paths`／安装器实际产物对齐，不能凭印象。

## 门禁状态

- `rustfmt --edition 2024`（写入 + `--check` 复检）：两文件干净。
- `cargo clippy --workspace --all-targets -- -D warnings`：干净（首轮报 2 处测试里的 `needless_borrows_for_generic_args`，已修）。
- `cargo test --workspace`：全绿（libscoop 852 passed / 9 ignored；新增 5 项测试：悬空 current、活 current 不误报、断面 shim、健康 shim 不误报、仅 `.exe` 不误报）。
- `node .agents/scripts/doc-links.mjs`、`docs-index-check.mjs`：断链 0、索引缺口 0。
