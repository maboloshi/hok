# `cat_style` 是 bat 的 `--style` 值：吞掉的退出码 + 写入期求证

> 2026-09-27 ｜ 1 提交 `95fb819` ｜ worktree `.`（主树直接改） ｜ 分支 `refactor/edition-2024`

**起因**：`hok cat scoop-app/winsw-mihomo-service` 只输出 `error: invalid value 'bat' for '--style <components>'`，**清单一个字没打印**，`exit=0`；用户配置里是 `"cat_style": "bat"`。

**根因（两层）**：
1. **代码**：`src/cmd/cat.rs` 的 `run_bat` 拿到 `child.wait()` 后**丢掉退出状态**、恒返回 `Ok(())` ⇒ `used_bat && run_bat(..).is_ok()` 里那句「bat 失败就退回纯 JSON」**是死代码**。bat 的参数解析失败发生在读 stdin 之前，`write_all` 的 `BrokenPipe` 也被 `?` 掩盖，于是既没有退回、也没有交接诊断。
2. **语义错位**：`cat_style` 是 **bat `--style` 的取值**（与官方 `scoop-cat.ps1:21` 同源），而 hok 的 README 与 `hok config -a` 描述写成"用 bat 显示清单 / Show manifests through bat"——`hok config cat_style bat` 读起来完全合理，bat 却直接 `exit 2`。文档描述写的是"这个开关做什么"，而它实际是"这个值是什么"。

**改动**：
- `cat`：bat 非 0 退出（或根本起不来）→ 一行 `WARN` 点名 `cat_style` + 退回纯 JSON；stderr 收进管道，警告里带 bat 原始诊断。
- `config`：`cat_style` 写入前**向 bat 求证**（`bat --no-paging --style <v> --language json`），拒绝则不写、exit 1；bat 不在 PATH 不拦（那时 `cat` 自己会报告）。
- `src/cmd/mod.rs` 新增共享 `bat_command()`：渲染与校验必须是同一条命令行，否则会出现「`config` 放行、`cat` 被拒」的漂移。
- README / README-zh / i18n / `config` long help 的描述全部改成"传给 `bat --style` 的组件"。
- `tests/cat_style_bat_cli.rs`：用 **hok 自身二进制的副本冒充 `bat.exe`** 当必败桩（含"桩确实会失败"自检），钉住退回、拒绝、bat 缺失时不拦三条语义。

**关键取舍——不冻结 bat 的组件表**：bat 0.26 的 `--style` 接受 `default/full/auto/plain/changes/header/header-filename/header-filesize/grid/rule/numbers/snip`，且支持 `+grid` / `-numbers` 这类增量写法，集合**随 bat 版本增长**。硬编码名单会把新版 bat 的合法值拒掉，所以改成问工具本身；失败样本、证据与推理见提交正文。

**验证**：`cargo test --workspace` 全绿（libscoop 789 passed / 0 failed、新增 4 条集成测试）、`cargo clippy --workspace --all-targets -- -D warnings` 干净、改动文件 `rustfmt --edition 2024 --check` 干净；真实环境实测三条路径（`cat_style=bat` → 清单 + WARN；`config cat_style bat` → 拒绝 exit 1；`config cat_style numbers,grid` → 接受）。收尾四件套：② 偏离审计 `command-deviation-audit-2026-09.md`（§5 cat 行更新 bat 处置、§6.3 新增写入期求证一条、三处行号刷新，随主题提交）；① 计划表无对应条目；③ 本条日志。

**坑/教训**：① **`io::Result<()>` ≠ 成功**：`Command::wait()` 的 `Ok(status)` 只表示"等到了"，任何"失败就退回"的分支都必须读 `status.success()`——对照 `hok-audit-fix-history` 里"事件必须闭环"同型。② 外部工具的**取值词汇表**别在自家硬编码，问工具本身（同型：git 的 `--date`、bat 的 `--style`）。③ 描述写错方向会**稳定制造**配置错误：写"这个值是什么"，而不是"这个开关做什么"。④ 环境注记：会话中途（01:35:25）用户 `~/.config/hok/config.json` 变成"`cat_style` 键消失、其余键原样"的 unset 形态，非本会话改动，已向用户报告待确认。
