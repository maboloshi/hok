# 任务：统一 hok --help 的结构性表头(中英混杂)与消除重复[选项]段

Status: closed
Archived: 2026-10-09

> 交接提示词：由另一个全新会话执行。你没有本对话上下文，请从下面内容 + 仓库自取证据。

## 目标(完成标准)
让 hok 每个子命令的 <cmd> --help 帮助文本在**同一语言内表头一致**：
1. 结构性表头(用法/子命令/参数/选项/帮助 等)**全中或全英**，不再同一屏混 Arguments:/Options:(英文)与 用法:/子命令:/选项:(中文)。
2. 不再出现**两个并列的选项段**(当前每个叶子命令的 --help 都有：英文 Options: 只含 -h，紧接着又一个中文 选项: 列出 -v/-q/-L/-N/--confirm 全局参数)。
3. -L en 与默认(zh)两种语言下都一致；改动不破坏现有功能与现有测试。

## 问题实况(已复现证据)
- 叶子命令 cat、prefix、depends 等：先英文 Arguments: <PACKAGE> 要检查的包名称，后 Options: -h,--help，再整段中文 选项:(-v -q -L -N --confirm)。
- 带子命令的 cache/bucket/shim 等：用法:/子命令:(中文)后面却是英文 Options:(只含 -h)，之后又一段中文 选项:。
- 复现：cargo build 后分别跑 hok cat --help、hok cache --help、hok prefix --help、hok --help，并对比 hok -L en cat --help。

## 机制与可能根因(先读这些再改)
- crates/hok-i18n-derive/src/lib.rs：过程宏 I18nHelp，编译期读 src/cmd/mod.rs 的 Command 枚举 + 每个 src/cmd/<x>.rs 的 Args/内层 Command，从 locales 的 cli_help: 组拉取各命令 about/参数文案，生成把 clap 树标头转成目标语言的帮助。怀疑：它只把[命令自带文案/某些标题]本地化，而 clap 自动生成的 Arguments:/Options:(-h 归属)标题未纳入翻译，于是结构头一半英一半中。
- src/cmd/mod.rs：顶层 Cli(#[command(subcommand_required = true)] 等)，字段 #[command(flatten)] verbose: Verbosity + #[arg(global = true, short = 'L', long = 'lang', ...)] language、-N/--no-color、--confirm。这些 global/flatten 参数叠加到每个子命令帮助，疑似形成第二个选项段(其标题被 i18n 设成中文[选项]，而 clap 自带 Options 只渲染了 -h)。
- locales/en.yml 与 zh.yml：cli_help:(约 256-627 行)与 cli_alias: 是帮助文案来源；cmd: 与 output: 是运行时消息。改文案/加标题翻译在此处。
- 架构：CLI 层 src/ 负责参数解析与输出渲染；libscoop 与 hok-i18n-derive 是子 crate。遵循 hok 五层单向依赖与[命名即文档]。

## 建议调查路线
1. 跑复现，逐项确认哪些标题英文/中文、来自哪里(Usage/Arguments/Options/Subcommands/选项…)。
2. 读 I18nHelp derive 全流程(尤其它如何给标题/参数设 help heading、如何拼 per-command 全局参数)，定位 Options:(-h)与 选项:(全局)为何分成两节——大概率是 next_help_heading/help_heading 与 clap 默认标题混用。
3. 决定统一方案(任选其一，优先最小侵入)：
   - 让 clap 的 Arguments/Options/Usage/Subcommands 标题也走 i18n 到目标语言；或
   - 关闭 clap 默认英文标题、改由 i18n derive 统一给全部结构头设本地化标题；
   - 把全局 -v/-L/-N/--confirm 归并进同一个选项段(与 -h 同标题)，消除重复节。
4. 挑代表命令回归：叶子(cat/prefix/info/which)、嵌套(cache/bucket/shim/config)、顶层 hok --help；en 与 zh 各一遍。
5. 评估：-h(short) 与 --help(long) 两条渲染路径都要一致。

## 硬性完成条件
- cargo check --workspace 与 cargo build 全绿；跑相关测试(若新增标题翻译无测试则手验)。
- 改完的 Rust 文件用 rustfmt --edition 2024 <file>(本仓全部 edition=2024)；**改动范围只限本次文件**，不整包 cargo fmt。
- 若改了 locales 的 cli_help:：en 与 zh 键一一对应；注意缩进/分组正确(曾有键插错顶层分组导致 zh 解析不到、回退成键名——插入新键务必放对分组且改后实际跑 zh 验证)。
- 不要引入补丁式开关掩盖副作用；若根因是 clap 标题机制，改机制本身。
- 提交前对照 git status 只暂存本次主题文件；分组提交、每个提交带 -S 签名(先 $env:HOME=$env:USERPROFILE，见 git-signed-commits：gpg 瞬时失败原样重试 2-3 次，勿关 gpgsign)。分支 refactor/edition-2024。
- 本仓库 git 命令统一加 -c core.autocrlf=false，工作树保持 LF。

## 范围外(不要顺手做)
- 不重构 I18nHelp 之外的其他 i18n 缺陷；不改各命令输出渲染。
- 不改 Scoop 语义/退出码；这是纯帮助文案一致性任务。

## 收尾
完成后：git log --show-signature 见 Good signature；更新 Hindsight 改进页 relates_to kp-b104a7e671584ce888a0d021d8981992(标题：复盘修复不常用子命令的输出样式与呈现偏差)标记该 M3 项完成。
