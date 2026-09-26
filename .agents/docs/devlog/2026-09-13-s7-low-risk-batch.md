# S7：低风险批量

> 2026-09-13 ｜ 文案 / 列名 / 列序 + 标志命名对齐，分支 `fix/s7s14-low-risk`

## 背景与决策

fix plan 批次 7 = S7（低风险 文案/列名/列序/默认值）+ S14 中属**输出类**的部分。用户拍板：T1+T2 全做、**P2（单 worktree 串行）**、`status` **保留** `Source` 列与 `:: Status of installed apps` 分节标题、`shim` 无子命令**保留**默认 list、`upgrade -f` 改为 force；**S14 的参数/退出码类划出另开会话**。实施过程中用户追加了 4 条命名对齐指令，最终把"同一件事两个名字"一并清掉。

## 实现

- **标志 / 内部命名**：`--skip-hash-check` 成唯一拼写（删 `--no-hash-check` 别名），内部同步 `skip_hash_check` / `SkipHashCheck` / locale 键；`--ignore-failure` 去掉短选项 `-f`（与官方 force 冲突），内部改 `ignore_failures` / `IgnoreFailures`，帮助文案写明 `hok config ignore_failures true`；`-D/--ignore-cache` → `-k/--no-cache`（内部 `no_cache` / `NoCache`，描述用官方 `Don't use the download cache`）；`upgrade -f` = force。
- **config**：三个 hok 自有设置键统一为下划线 `ignore_failures` / `no_color` / `script_engine`，并**删除全部 serde 别名**；`canonical_key` 单表仍是唯一入口，`aria2-*` 的 `serde(rename)` 与 `match` 保留。
- **输出类**：`status` 列名 / 分隔 / 全好文案；`list` 表头回显查询 + `Updated` 列 + Info 词按命令分语境（`status_info_word(kind, InfoContext)` 单入口）；`cache rm` 逐文件、`cleanup` 逐版本；`search` 无查询列出全部；`download -f` 提示 `Cache is being ignored.`；`reset <未安装>` exit 0；`update` 逐应用最新版本行 + `--no-cache` 无 `<app>` 报错；`install` 已安装文案带版本（`prune_installed` 改回 `(name, version)`）；`bucket` 列序 / 排序 / `known`；`shim list`/`info` 对象形状。

## 验证

隔离 `SCOOP`/`HOME` 根，`BEFORE`（base `cf1baec` 构建）与 `AFTER` 实测 `status` 前后输出（报告 §9.1）；`cargo test --workspace` 仅 2 条 ACL 环境性失败（clean HEAD 同样失败）；`--test i18n_keys|cli_help_i18n|exit_codes` 全绿；`clippy --workspace --all-targets --exclude innospect --no-deps -- -D warnings` 干净；文件级 `rustfmt` 干净。

## 教训

1. **改名要连"内部名 + locale 键 + 枚举"一起改，否则半新半旧**：`--skip-hash-check` 首版只动了长选项名，内部仍叫 `no_hash_check`、i18n 键也照旧——用户要求"内部也别用"才一次改净。i18n 键名取自 **Rust 字段名**（`hok-i18n-derive`），改字段必须同步改两个 yml，否则 `cli_help_i18n` 报 raw key。
2. **"加回统一样式"也是决策**：`status` 的分节标题一开始按"官方裸表"删掉了，用户要 hok 的 `:: <标题>` 统一样式加回 ⇒ "对齐官方"与"保持 hok 观感一致"之间要显式问，不能默认前者优先。
3. **YAML 值里有 `: ` 必须加引号**：`ignore_failure: … (persistent setting: 'hok config …')` 让 `hok-i18n` 的 build.rs 直接 panic（mapping values are not allowed in this context）；给值加引号即解。
4. **别名清理要区分"数据格式兼容"与"内部命名"**：config 的 `serde(alias)` 是**配置文件**里的旧拼写，删掉等于让旧配置静默失效（登记为破坏性变更）；而 `aria2-*` 的 `serde(rename)` 是官方拼写、必须留。用一条 `alias` 正则整片删会误伤。
5. **别把 `serde(rename)` 当 no-op 删**：首版按"`-`→`_` 后与字段同名即冗余"的规则删 `rename`，把 `aria2-enabled` 这类**有意义的**重命名一起删了；改成**精确相等**才算冗余，并 `git checkout` 重做。
6. **门禁要能区分既有噪声**：本仓内置 `innospect` 在新版 clippy 下有既有告警，`build.rs`/`hok-i18n-derive/src/lib.rs` 有既有 rustfmt diff ⇒ 报告"我改的文件干净 + `--exclude innospect` 的 clippy 干净"比笼统说"clippy 失败"有用得多。

---
