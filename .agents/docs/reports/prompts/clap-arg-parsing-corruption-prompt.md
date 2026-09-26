# 提示词：修复 clap 参数解析错位（`hok checkver` / `hok reset`）

> **用法**：在新会话里把本文件作为首要输入（`@.agents/docs/reports/prompts/clap-arg-parsing-corruption-prompt.md`）。
> 这是一份**自包含**交接：新会话没有本议题的历史上下文，全部依据写在这里。
> **前置加载的 skill**：`hok-rust-gate-practice`、`git-commit-practice`、`hok-proactive-review`、`token-efficient-processing`、`no-patchwork-fixes`。

---

## 0. 一句话目标

`hok checkver` 与 `hok reset` 的参数解析整体错位（帮助、`reset -a`、`checkver --version/--timeout` 全部受影响）。根因已定位到 `src/cmd/mod.rs` 的 `localize_structural_headings`：它在 `Command::build()` **之后**调用 `Command::mut_arg`，而 clap 4 的 `mut_arg` 是「删 + 推到末尾」，会让 clap 缓存的「参数名 → 下标」映射失效。修掉它，并加一个能挡住回归的全命令参数面测试。

## 1. 症状（已实测，可直接复现）

### 1.1 帮助面（50 个 surface × 2 locale × 2 flag = 200 次探测，**恰好 8 个坏**）

| surface | 结果 |
|---|---|
| `hok checkver --help` / `-h`（en、zh） | `error: a value is required for '--version <VERSION>' but none was supplied`（exit 2） |
| `hok reset --help` / `-h`（en、zh） | `error: a value is required for '[VERSION]' but none was supplied`（exit 2） |
| 其余 49 个 surface（36 命令 + 嵌套子命令，含根命令） | 正常 |

`hok help checkver` 与 `hok help reset` 是**正常**的——它们走 clap 的 help 子命令渲染路径，不经过 `mut_arg` 后的解析。别用它来证明「没问题」。

### 1.2 功能面（比帮助更严重）

```
$ hok reset -a            # 官方 Scoop：reset 全部应用
error: a value is required for '--lang <LANGUAGE>' but none was supplied
$ hok reset --all
error: a value is required for '--lang <LANGUAGE>' but none was supplied
$ hok reset --help x      # 静默把 x 当成 version 吃掉，然后报「缺少 <app>」
ERROR 缺少 <app>。使用 'hok reset <app>' 或 'hok reset -a' 重置所有应用。

$ hok checkver --version abc     # 用 timeout(u64) 的解析器
error: invalid value 'abc' for '--timeout <TIMEOUT>': invalid digit found in string
$ hok checkver --timeout abc     # 用 lang(ValueEnum) 的解析器
error: invalid value 'abc' for '--lang <LANGUAGE>'
$ hok checkver --lang abc        # 当成 dir 吃掉了
ERROR 错误: 'abc' 不是目录
$ hok checkver --skip-updated abc
ERROR 错误: 'abc' 不是目录
```

即 `checkver` 的长参数查找整体错位一格以上，`reset -a/--all` 完全不可用（`reset <app>`、`reset` 无参、`checkver -u/-f` 这类仍正常）。

### 1.3 与本次 i18n 改动无关（有硬证据）

- `target/i18n-verify/before/`（用**改动前**的二进制、在 `4fc01f9` 上采集）里 `en/checkver.txt`、`en/reset.txt` 就是同样的两条报错，且改动后 `after/` 与之**逐字节一致**。
- `git diff 4fc01f9..HEAD -- src/cmd/mod.rs` 除 `t!` 改名外，只有一行 `-use rust_i18n::t;`，`localize_*` 的逻辑一字未动，参数改动顺序与改动前完全相同。
- 结论：**纯历史遗留缺陷**，与 `.agents/docs/reports/prompts/i18n-inhouse-replacement-prompt.md`（第 4 项，已由 `c99f07e`/`cb34afb`/`84fe376`/`c0dd4a1` 完成）无关；当时为保住「用户可见文案零变化」的验收口径而未动。

## 2. 根因（已读 clap 源码确认，非猜测）

三个事实叠加：

**(1) `Command::mut_arg` 不保序、且不维护 key 映射。**
`clap_builder-4.6.6/src/builder/command.rs:244-256`：

```rust
let a = self.args.remove_by_name(id).unwrap_or_else(|| panic!("Argument `{id}` is undefined"));
self.args.push(f(a));
```

底层 `MKeyMap`（`src/mkeymap.rs`）：

```rust
pub(crate) struct MKeyMap { args: Vec<Arg>, keys: Vec<Key> }   // Key { key: KeyType, index: usize }
pub(crate) fn push(&mut self, new_arg: Arg)   { self.args.push(new_arg); }              // 不碰 keys
pub(crate) fn remove_by_name(&mut self, name: &str) -> Option<Arg> {
    self.args.iter().position(|arg| arg.id == name).map(|i| self.args.remove(i))        // 不碰 keys
}
pub(crate) fn get<K>(&self, key: &K) -> Option<&Arg> {                                  // 查找走 keys 里的缓存下标
    self.keys.iter().find(|k| &k.key == key).map(|k| &self.args[k.index])
}
```

**(2) `keys` 只在 `_build_self` 里生成，而 `build()` 是一次性的。**
`command.rs:4400`：

```rust
if !self.settings.is_set(AppSettings::Built) {
    ...
    self.settings.set(AppSettings::Built);
} else {
    debug!("Command::_build: already built");   // 第二次 build() 是 no-op，不会重建 keys
}
```

`command.rs:4388` 的 `build()` 还是**递归**的（`_build_recursive` 会连子命令一起置为 Built），所以「只对根 build」也会把子命令的 keys 一起冻结。

**(3) hok 恰好是「先 build 再改参数」。**
`src/cmd/mod.rs`：

```rust
fn localize_structural_headings(mut cmd: clap::Command) -> clap::Command {
    let args_h = ...; let opts_h = ...;
    cmd.build();                                        // ← 冻结 keys（含所有子命令）
    localize_heading_level(cmd, &args_h, &opts_h)       // ← 里面全是 mut_arg（删+推）
}
```

### 2.1 为什么 49 个命令没事，只有 checkver / reset 崩

`localize_heading_level` 先按 `get_arguments()` 顺序对**所有** positional、再对**所有** option 逐个 `mut_arg`。同序「删+推」是一次整体旋转，处理完全部参数后 Vec 顺序**恢复原状**（positionals 被提前，其余相对顺序不变），于是陈旧的 `keys` 下标**碰巧**仍然正确——这就是它能跑这么久的原因。

真正造成位移的是紧随其后的特殊循环：

```rust
for special in &["help", "version"] {
    if cmd.get_arguments().any(|a| a.get_id() == *special) {   // ← 只看 id，不看 action
        cmd = cmd.mut_arg(*special, ...);
    }
}
```

- 自动生成的 `-h/--help` 在 build 时被追加在**末尾**，`remove+push` 对末尾元素是恒等操作 ⇒ 无事。
- 但 `version` 这个 id 在 **checkver 和 reset 上是「用户自己定义的参数」**（全仓仅此两处）：
  - `checkver`：`#[arg(short='V', long)] version: Option<String>`（在第 5 位）
  - `reset`：`version: Option<String>` 位置参数

  于是 `mut_arg("version")` 把这个**不在末尾**的参数推到末尾，`keys` 却仍记着旧下标 ⇒ `--help/-h` 查到的不再是 help，而是被挪过来的那个参数：

  - checkver：`keys[help] = 7`，而 `args[7]` 现在是 `version` ⇒ clap 认为 `--help` 命中 `--version` ⇒ `a value is required for '--version <VERSION>'`（与实测报错逐字对应）
  - reset：同理命中位置参数 `version` ⇒ `a value is required for '[VERSION]'`
  - reset 的 `-a/--all` 则被位移撞到别的槽位（实测报 `--lang`）

  `--lang abc` / `--skip-updated abc` 被当成 `dir`、`--version` 用 timeout 的解析器等，都是同一份 `keys` 整体错位的表现。

**第二个独立缺陷（同一段代码）**：`cli_help.hok.args.version`（clap 自动 `--version` 的文案 = "Print version"）被套到了 checkver **自己的** `--version` 上，所以 `hok help checkver` 里显示：

```
  -V, --version <VERSION>
          Print version          # 应为 "Update manifest to specific version (skip version detection)"
```

## 3. 修复方向（建议，允许合理偏离但要写明理由）

**核心原则（必须遵守）**：`Command::mut_arg` / `mut_subcommand` 只能在 `build()` **之前**调用；一旦 build 过，就不要再改参数。

建议按此顺序：

1. **把 `build()` 挪到最后**：`localize_structural_headings` 里去掉提前的 `cmd.build()`，改成「先做所有 mutation，再 build」（或干脆不显式 build，交给 `get_matches_from` 内部 build）。
   代价：根命令的自动 `-h/--help`、`-V/--version` 在 build 前**不存在**，`localize_heading_level` 现靠它们存在来设置 `help_heading`（否则 clap 会渲染英文 "Options:"/"Arguments:"）。可选做法：
   - a. 对这两个自动参数改为 `build()` 之后**单独 patch，且只在它们确实位于 Vec 末尾时**做（末尾 `remove+push` 恒等，安全），并加注释说明为什么只对末尾安全；
   - b. 或放弃 patch 自动参数，改用 `disable_help_flag(true)` + 自建 `--help` 参数（表面对齐会更可控，但要核对 `--help` 输出不漂移）；
   - c. 或先 `Cli::command()` 建两棵树：一棵只用于读取自动参数的名字/位置，一棵真正 mutate。
2. **特殊循环改按 action 识别自动参数**，不要按 id：
   ```rust
   use clap::ArgAction;
   let is_auto = |a: &clap::Arg| matches!(a.get_action(), ArgAction::Help | ArgAction::Version);
   ```
   这样 checkver 的用户 `--version`、reset 的位置参数 `version` 都不会再被误当成 clap 的自动参数（顺带修掉 §2.1 末尾那个文案错误）。
3. **考虑上游**：给 clap 提 issue/PR——`mut_arg` 在 `build()` 之后使用会静默破坏 `MKeyMap::keys`（至少该 `debug_assert!(!Built)` 或在 mutation 后清掉 `Built` 标记）。
4. 顺带检查是否有**其他**「build 之后 mutate」的地方：全仓搜索 `\.build\(\)` 与 `mut_arg`/`mut_subcommand`/`mut_group` 的调用点（本次只发现了 `localize_structural_headings`）。

## 4. 验收条件（全部必须通过）

1. **帮助面全绿**：50 个 surface × {en, zh} × {`--help`, `-h`} 全部 exit 0 且输出非空。
   现成脚本：`target/i18n-verify/help-audit.js`（本次实测 200 探测 → 8 坏）。建议把它落成 `tests/cli_help.rs`（用 `assert_cmd`/`std::process::Command` 起自己的二进制，`env!("CARGO_BIN_EXE_hok")`）以便长期挡回归。
2. **功能面**（手工 + 尽量自动化）：
   - `hok reset -a` / `hok reset --all` 不再报参数错误（会真的执行重置，测试里请用隔离的 root，或只断言「错误不是 clap 的参数错误」）；
   - `hok checkver --version 1.2.3` 的值进入 `version`（不再被 u64 解析器拒），`--timeout 60` 进入 `timeout`，`--lang en` 进入 `lang`；
   - 反例守卫：`hok checkver --version abc` **不应**报 `'--timeout'`。
3. **文案纠正**：`hok help checkver` 里 `-V, --version <VERSION>` 的文案来自 `cli_help.checkver.args.version`，不是 `cli_help.hok.args.version`("Print version")。
4. **不回归**：其余 49 个 surface 的帮助文本**逐字节不变**。
   采集办法（本次已验证可用）：`node target/i18n-verify/dump-help.js target/debug/hok.exe <out-dir>`，改动前后各跑一次做原始字节比对。注意它会把 `checkver`/`reset` 的失败也原样记下来，修好后这两处**会**变化，属于预期差异。
   该 dump 的 8 处预期变化之外，任何差异都要查清。
5. **门禁**：`cargo check --workspace --all-targets` + `cargo clippy --workspace --all-targets`（只允许余 vendored `innospect` 的 4 条）+ `cargo test --workspace`（当前 lib 515 passed）。

## 5. 陷阱（血泪点）

- **别在 `build()` 之后 `mut_arg`**——本题根因，改完顺手在注释里写明原因，防止后人再犯。
- `Command::build()` **递归**，会把所有子命令也标记为 Built；「只 build 根」不等于「只冻结根」。
- `mut_arg` 的 id 不存在会 **panic**（`Argument \`{id}\` is undefined`）。
- `hok help <cmd>` 正常不代表 `<cmd> --help` 正常（两者渲染路径不同），必须分开验。
- `--help`、`-h` 走的是不同 key（`Long("help")` / `Short('h')`），**两个都要测**；本次两者同时坏，只测一个会漏。
- 帮助文本是本地化的，脚本里解析标题要固定 `--lang en`（zh 下是「子命令:」），否则会误判为「没有子命令」。
- 别用 `hok checkver` / `hok reset -a` 直接做手工验证——它们会真的跑网络/改本机 Scoop 目录；用 `--dir` 指向临时目录或不存在的目录来短路。
- **不要整包 `cargo fmt`**：仓库存在既有 fmt diff（本题改动的 `src/cmd/mod.rs` 就在其中），只 fmt 本次改动文件；见 skill `hok-rust-gate-practice`。
- **git 一律 `-c core.autocrlf=false`**；本机 `commit.gpgsign=true` 自动签名。
- 提交信息用 `git commit -F <文件>`，**别用 PowerShell here-string 直接 `-m`**：双引号 here-string 里反引号是转义符，markdown 反引号会把 `` `t ``→TAB、`` `r ``→CR、`` `n ``→LF，把提交信息写坏（本次踩过）。

## 6. 建议的提交拆分

主题单一，建议 **2 个提交**：

1. `fix(cli): build the clap command tree after patching it`（`src/cmd/mod.rs`，含根因注释；修 checkver/reset 解析错位 + 自动参数识别改按 action）
2. `test(cli): assert every command's --help exits cleanly`（新增全命令帮助面测试，把 `help-audit.js` 的逻辑落成 Rust 测试）

若第 1 步不可避免地改变了某些帮助文本，把「文案纠正」单独拆一个提交。

## 7. 参考：本次已产出的可复用素材

| 路径 | 说明 |
|---|---|
| `.agents/scripts/help-audit.js` | 枚举全部真实帮助面并报告失败：`node .agents/scripts/help-audit.js <hok.exe>`（当前输出：`surfaces: 50  probes: 200  broken: 8`） |
| `.agents/scripts/dump-help.js` | 逐面 dump 到目录树，供改动前后原始字节比对：`node .agents/scripts/dump-help.js <hok.exe> <out-dir>` |
| `target/i18n-verify/before/` `after/` | 本次 i18n 改动前后的 dump（`4fc01f9` 的旧二进制 vs 新二进制），含 checkver/reset 的失败记录；在 `target/` 下，`cargo clean` 会清掉 |

> 提示：`dump-help.js` 会把 `checkver`/`reset` 的失败也原样记下来（带 `<<<exit N>>>` 头），修好后这两处**会**变化，属于预期差异。它末尾还有一层 `\(built [^)]*\)` 归一化——但本次实测两次构建的 `BUILD_DATE` 相同（build.rs 未重跑），所以实际比对是**未归一化的原始字节**。

## 8. 背景（为什么现在做/当时没做）

本议题是在执行 `.agents/docs/reports/prompts/i18n-inhouse-replacement-prompt.md`（优化审计第 4 项「替换 rust-i18n」）时，用帮助面 before/after 逐字节比对**顺带发现**的：

- 那次改动要求「用户可见文案零变化」，验收手段是把 63 个帮助面 dump 下来做原始字节 diff；
- dump 时 `hok checkver --help` 直接报错，进一步排查发现 `reset` 也一样，且不止是帮助、功能也受影响；
- 但它与 i18n 无关且**先于**本次改动存在，修它会改变帮助输出（从报错变成正常帮助），会污染「零变化」口径，因此当时只做记录、交接给独立会话处理。
- 审计报告对应小节：`.agents/docs/active/audit/optimization-audit-2026-09.md` §9.4。
