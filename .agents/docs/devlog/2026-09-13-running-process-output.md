# 运行中进程输出 + `ignore_running_processes` 语义回正

> 2026-09-13 ｜ 会话 `running-process-output`

**背景**：用户实测 `hok update && hok upgrade` 拿到 `ERROR 'PowerShell' 仍在运行！请关闭该应用后重试；跳过更新。` 外加紧跟的四行 `pwsh (pid …)`。三点诉求：① 用 WARN 不用 ERROR；② 文案回到 `Running process detected, skip updating 'x'.`；③ 进程名+pid 只在 `-vv` 输出、样式改成缩进 dim 行。用户拍板：`ignore_running_processes=true` 时=**继续操作**（"组合 1"），该键文案用两行的 C3。

**改了什么**（`sync_install.rs` 为核心）：
- `RunningCheck` 恢复三态：`NotRunning` / `Running`（默认 skip）/ `Ignored`（按配置继续）；`Error::AppRunning` 不复活。这是对 `a4ea0b7` 的**语义回正**——官方 `test_running_process` 的 ignore 分支是 `return $false`（= 继续），不是"仍在运行"。
- 新增 `RunningAction`（`Update`/`Reinstall`/`Uninstall`/`Reset`）取代 `action: &str` 形参：英文动名词无法从裸动词推导，故每个动词一个 i18n 键（`cmd.running_process_{update,reinstall,uninstall,reset}`）；`action.*` 四个键随之成为死键并删除。
- 消息选择收进 `check_not_running`：默认 `Running process detected, skip updating '%{name}'.`（WARN）；开键 `cmd.running_process_ignored` 两行 WARN（第二行的 5 空格缩进**写在 locale 里**，对齐渲染器加的 `WARN ` 前缀）。
- 进程明细不再拼进消息：新增 `Output::Detail` + `OutputHandle::detail` + CLI `output::detail(msg)`（`  {dim}`，**stderr**，与所属 WARN 同流），由全局 `-vv`（`LevelFilter::INFO`）闸门渲染；`-v` 及以下不出现。库侧仍无条件发射，保持 verbosity-agnostic。
- 顺序必须**消息在前、明细在后**（首版写成明细在前，冒烟实测抓到后修正；单测改为有序事件流钉住顺序）。

**验证**（临时 `SCOOP` 根 + 假安装 + 真跑 `app.exe`，再跑 `hok uninstall app`）：
- 默认（无 `-v`／`-v`）：`WARN 检测到进程正在运行，跳过卸载 'app'。`（en：`WARN Running process detected, skip uninstalling 'app'.`），无明细；
- `-vv`：同一条 WARN + `  app (pid N)`；
- `ignore_running_processes=true`：两行 WARN（含"可能导致数据丢失或应用不稳定，建议先关闭再重试"）+ 明细，随后**真的继续**卸载，并因 exe 被占用而 `ERROR … it may be in use: 拒绝访问。(os error 5)` —— 正是该警告所述风险。

**门禁**：`cargo test -p libscoop -p hok` → hok 侧全绿，libscoop lib 742 passed / 2 failed（仅既有 `permission::tests::grant_users_write_inherit_*` 的 ACL `error 5` 环境性失败）；`cargo clippy --fix -p libscoop -p hok` 干净；改动文件级 `rustfmt --edition 2024` 干净。

**How to apply**：改运行中进程的文案/明细只动 `check_not_running`（`crates/libscoop/src/package/sync_install.rs`）+ 那 5 个 `cmd.running_process_*` 键；`-vv` 级明细统一走 `Output::Detail` → `output::detail`（stderr），别再把列表拼进消息；`ignore_running_processes=true` 是**真的继续**，不要再把它当"只改文案"的开关。附带注意：`-vv` 同时会打开 tracing 的 INFO 日志（既有行为），所以 `-vv` 下会先看到 `INFO libscoop::session: config loaded from …`。


### 2026-09-13 「Everything is ok!」页脚删除（同一会话 `running-process-output` 的第二件事）

**背景**：运行中进程被跳过时仍打印 `✓ 一切正常。`，用户指出该页脚本身多余。核实：官方**只有 `scoop status`** 打 `success 'Everything is ok!'`（`Scoop/libexec/scoop-status.ps1:80`），install/uninstall/update 都在 per-app 成功行结束；hok 这行是 2026-08-13 自加（CHANGELOG `6fe4b8e`），一直登记为"有意偏离，保留"。

**做了什么**：删除 `install.rs` / `uninstall.rs` / `upgrade.rs` / `reinstall.rs` 末尾的 `output::done(t!("output.ok_all"))`；`status.rs:161` **保留**（官方确有）。只为该页脚而存在的守卫一并清掉：`uninstall.rs` 的 `any_installed`（连同整块 `query_installed` 调用）；`upgrade.rs` 的 `missing_target` **必须保留**——它还守着 "All apps are up to date" 的早返回（否则未安装的具名目标会被误报成"已最新"），只删末尾的 `if missing_target { return }` + 页脚；`install` 的 `downloaded_only` 页脚保留（`--download-only` 专用）。

**为什么不走 `scoop_handler.rs` 屏蔽**：`scoop_handler.rs` 只渲染事件总线上的 `Event`（`PackageCommitDone → closure_done` 等），这行是各命令末尾**直接** `output::done(...)` 打的，根本不经过它；在那里屏蔽 = 借不相干接缝掩盖消息（[[no-patchwork-fixes]]），且 4 个发射点照旧"声称一切正常"。正确入口就是发射点本身。

**验证**：临时 SCOOP 根实测 `hok uninstall app` 成功路径收在 `✓ app`（官方 per-app 行）、无页脚；`hok status` 仍收 `✓ 一切正常。`。门禁：clippy 干净、`cargo test -p libscoop -p hok` 742 passed / 2 既有 ACL 失败。
