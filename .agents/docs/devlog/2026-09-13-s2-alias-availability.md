# S2 `alias` 可用性

> 2026-09-13 ｜ 已闭环，提交 `3c90095`，分支 `fix/alias-availability`

**决策**：方案 **A 官方式脚本别名**（用户 2026-09-13 拍板），附带项全选：重名 add / 不存在 rm 报错 exit 1、拒绝内建重名、去 60 字符截断、裸 `alias` 保留 list 并登记有意偏离。
**实现**：`crates/libscoop/src/alias.rs` = alias 生命周期唯一入口（`add`/`remove`/`list`/`resolve`/`run`/`script_path`）；CLI 只做参数与表格。config `alias` 值 = `"scoop-<name>"`（脚本名，官方语义），**命令正文在 `<shims>\scoop-<name>.ps1`**。原 `config::alias_add`/`alias_remove` 已删除。
**关键实现点（勿踩）**：
- 脚本字节必须与官方一致：`format!("# Summary: {desc}\n{command}\r\n")`，UTF-8 无 BOM（官方 `lib/commands.ps1:65-71` 的 `"`n"` join + `Out-UTF8File`→`WriteAllLines` 追加平台换行 ⇒ 内嵌 LF + 末尾 CRLF）。
- **分发只查文件**：`<shims>\scoop-<token>.ps1` 存在即执行（官方 `command_files`/`command_path` 同理），config 仅索引；因此 `<BROKEN>` 条目自然不可调用（与官方一致），不做 config 兜底。
- 拦截点在 `src/cmd/mod.rs` 的 `try_get_matches_from` **错误路径**（`ErrorKind::InvalidSubcommand`），happy path 不变；参数拆分 `alias_invocation` 必须跳过取值的选项（`--lang zh foo` 要得到 `foo`）。非别名回落 `err.exit()`，信息与 exit 2 与旧二进制逐项一致。
- **别名脚本会出现在 `hok shim list` 里——这是对齐官方**（`libexec/scoop-shim.ps1` 的 list 本就 `Get-ChildItem -Include '*.shim','*.ps1'`），**不要**为此加排除逻辑。
- `rm`/`s`/`u` 是 hok 的 clap 顶层别名 ⇒ 官方示例 `scoop alias add rm '...'` 在 hok 会被拒绝（`hok rm` 已存在），属有意为之。
**仍未做（有意偏离）**：裸 `alias` 默认 list（官方 exit 1）；空表文案 `No aliases configured.`（官方 `No alias found.`）；`alias rm` 成功文案 hok 自有；`hok help <alias>` 不渲染别名帮助；无别名自递归保护（官方也无）。
**Why:** 别名"看起来能用实际不能用"是最容易误导用户的语义缺陷，现已闭环；此条也是 `command-deviation-fix-plan` S2 的验收依据。
