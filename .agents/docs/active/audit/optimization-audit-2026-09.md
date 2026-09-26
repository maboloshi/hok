# hok 精简 / 合并 / 优化空间审计（2026-09-10）

> 方法：第三方工具实测 + 人工核验。原始产物在 `.agents/tmp/`（gitignore），脚本在 `.agents/tmp/*.py`。
> 工具：`cargo clippy 0.1.98`、`cargo-modules orphans`、`cargo-machete`、`drywall`(AST 克隆)、
> `jscpd 5.2.0`、`cargo tree -d`、`cargo metadata` 解析、`crates.io API`（上游版本/依赖对比）。
> `cargo-udeps` 因本机 nightly 工具链损坏未能运行（`missing manifest in toolchain`），已用 `cargo-machete` + 人工 grep 替代。

## 0. 规模基线

| 单元 | 生产行 | 测试行 | 测试占比 | 文件 |
|---|---:|---:|---:|---:|
| `crates/libscoop` | 26,314 | 10,971 | 29.4% | 85 |
| `crates/innospect`（内联 fork） | 12,220 | 1,494 | 10.9% | 50 |
| `src/`（hok CLI） | 5,808 | 217 | 3.6% | 47 |
| `crates/hok-shim` | 1,173 | 197 | 14.4% | 4 |
| `crates/hok-i18n-derive` | 292 | 0 | 0% | 1 |
| `tests/` | 263 | 0 | 0% | 1 |
| **合计** | **46,130** | **12,879** | **21.8%** | **189** |

## 1. 先说结论：主代码库已经很干净

| 检查 | 结果 |
|---|---|
| `cargo-modules orphans`（pub 项无人引用） | `libscoop`、`hok` 均 **0 orphan** |
| `drywall`（AST 级函数克隆） | 主 crate（`src` + `libscoop` + `hok-shim` + `i18n-derive`）仅 **10 组**；104 组中 73 组集中在 vendored `innospect` |
| `jscpd`（文本克隆，≥12 行/70 token） | **1.55%**（878/56,754 行，51 处）；且绝大多数落在 `#[cfg(test)]` 之后（如 `script.rs:783` 起的测试） |
| clippy（`--workspace --all-targets`） | 13 条 warning，约 10 类，全部可自动/机械修复 |

即：**"为了去重而去重"没有空间**；真正有价值的是下面 4 类。

## 2. P1 — 立即可做的精简（低风险）

### 2.1 未使用依赖 3 个（cargo-machete + 人工核验）

| 位置 | 依赖 | 证据 |
|---|---|---|
| `crates/hok-i18n-derive/Cargo.toml` | `proc-macro2` | 全 crate 0 次引用（只用 `proc_macro::TokenStream` + `quote`） |
| `crates/innospect/Cargo.toml` | `goblin` | 仅出现在注释（`overlay/pe.rs:83`） |
| `crates/innospect/Cargo.toml` | `encoding_rs` | 仅出现在注释（`records/language.rs:99/121`） |

`md-5`（libscoop / innospect）是 cargo-machete **误报**：两处都用 `use md5::…`，包名 `md-5` 与 lib 名 `md5` 不一致导致漏判。

收益：去掉 `goblin`（含 `scroll`/`plain` 等传递依赖）可缩短冷编译时间；对二进制体积影响很小（LTO 会剔除死代码）。

### 2.2 i18n 死键与缺失键

- `locales/en.yml` / `zh.yml` 中**整键在代码里完全不出现**（排除 `cli_help.*` / `cli_alias.*` 这类被 `hok-i18n-derive` 动态消费的键）共 **18 个**：
  `output.detail_prefix`、`output.select_one`、`status.resolving_done`、`status.sizing_done`、`status.downloading_done`、
  `detail.extract_done`、`detail.extracting_done`、`cmd.list_install_failed`、`cmd.list_bucket_removed`、`cmd.list_held`、
  `cmd.alias_add_usage`、`cmd.alias_rm_usage`、`cmd.alias_unknown`、`cmd.checkhashes_rehashed`、`cmd.checkver_version_wildcard`、
  `cmd.err_hash`、`cmd.shim_info_usage`、`cmd.computing_hash`。
- `zh.yml` 缺 3 个键：`cli_help.shim.long_about.HINT`、`.Options`、`.Subcommands`（英文有、中文无 → 中文帮助回退英文）。
  **更正（2026-09-10 执行时核实）**：这 3 个不是键，而是 `long_about: |-` 块标量里的正文行（`Subcommands:` / `Options:` / `HINT:`），
  被键解析脚本误判为键；中文块用的是「子命令: / 选项:」故被误报缺失。**zh.yml 实际无缺失键**，en/zh 键集一致。
- 键总量：en 587 / zh 584；`cli_help` 288 个键由 derive 动态使用，不能按"字面引用"判死。

### 2.3 clippy 13 条（多为 `--fix` 可自动修）

`chunks_exact` 常量块（4 处，建议 `as_chunks`）、`redundant pattern matching`（`internal/archive.rs:1192`）、
可折叠 `if`（`archive.rs:1338`）、可省略生命周期（`download.rs:120`）、`usize as usize`（`download.rs:561`）、
复杂类型（`download.rs:703`）、`clone` 造切片（`manifest.rs:1791/1801`）、多余解引用（`cmd/cat.rs:97`）、
`needless return`（`cmd/info.rs:53`）、手写检查除法（`cui.rs:255`）、未用 import `sha1::Digest`。

### 2.4 `constant.rs` 的 `#![allow(dead_code)]` 已无必要

`crates/libscoop/src/constant.rs` 现有 9 个 `pub static`（+1 私有 static），**全部被引用**；模块级 `#![allow(dead_code)]`
只剩"掩盖未来死代码"的作用，建议移除（若确有为 Scoop 对齐而保留的常量，改为逐项 `#[allow(dead_code)]` 并注明原因）。

## 3. P2 — 合并空间

### 3.1 最大杠杆：`crates/innospect` 内联 fork（12,784 行 ≈ 生产代码 21%）— ✅ 已完成（2026-09-19）

> **结局**：上游 `innospect 0.1.4`（2026-09-18）收录了 PR #3（`lzma-rust2` + `flate2/zlib-rs`，@maboloshi），
> 并额外把 LZMA2 字典大小改为读 Inno 的属性字节（修掉 fork 里固定 32MiB 的限制）。
> hok 侧已按 nsis 先例退役 fork：`libscoop` 改用 registry `innospect = "0.1.4"`，`crates/innospect` 整个删除
> （分支 `chore/innospect-registry`）。取舍：不再随 workspace 跑 innospect 自带的 79 个测试；
> 上游声明的 `goblin`/`encoding_rs` 回到构建图（本地 fork 曾删掉这两个未用依赖）。
> 下面的原始分析保留作记录。

- 本地 `crates/innospect` 与 crates.io `innospect 0.1.3` 逐文件 diff：**49 个文件全同，仅 2 个文件有差异**
  —— `src/decompress/block.rs`(+24/−18)、`src/extract/chunk.rs`(+19/−12)，改动就是解码后端换成 `lzma-rust2` + `flate2/zlib-rs`（外加 Cargo.toml 依赖调整）。
- 上游仓库 `ATRAPSLLC/innospect`，最新版仍是 0.1.3（与本地基线相同），所以 fork 目前是必要的。
- **建议走 nsis 的既有路线**：把这个 73 行 patch 上游化（或请上游提供 decoder backend feature），然后改回 registry 依赖，
  直接删掉整个 `crates/innospect`。收益：仓库生产代码少 21%、少一份 fork 维护负担、CI 少编译一个 crate。
- 期间可做的过渡动作：`crates/innospect/README.md` 或 `Cargo.toml` 注明 fork 差异与上游追踪链接（当前 fork 理由只写在 `libscoop/Cargo.toml` 注释里）。

### 3.2 cmd 层 clap 参数重复

39 个 `src/cmd/*.rs`（3,550 行，均值 91 行），执行参数已用 `SyncFlags`/`SyncArgs` 收敛，但 **clap 字段仍逐文件重复**：

| 字段 | 出现在几个命令 |
|---|---:|
| `global: bool` | 9 |
| `package: Vec<String>` | 8 |
| `app: Vec<String>` / `all: bool` / `no_hash_check: bool` | 6 |
| `ignore_failure: bool` | 5 |
| `package: String` / `force` / `offline` / `assume_yes` / `escape_hold` | 4 |

可抽 `#[derive(clap::Args)]` 的参数组 + `#[command(flatten)]`（如 `GlobalFlag`、`SyncCommon`），减少约 60 处字段声明，
并消除"某命令漏加 `--global`/文案漂移"的风险。代价：各命令 help 文案/顺序可能需微调，属可接受。

### 3.3 `[workspace.dependencies]` 统一跨 manifest 声明

14 个依赖在多个 manifest 重复声明（同上下文）：`bzip2`、`flate2`、`lzma-rust2`、`md-5`、`regex`、`rust-i18n`、
`sha1`、`sha2`、`tokio`、`tokio-util`、`tracing`（+ `serde_json`/`time` 分属 build-dep 与 normal dep，不算冲突）。

其中 **`flate2` 的 feature 必须跨 crate 保持一致**（libscoop 写 `"1"` + `zlib-rs`，innospect 写 `"1.1.9"` + `zlib-rs`）：
一旦某个 crate 漏掉 `zlib-rs`，feature 统一会重新把 `miniz_oxide` 拉回依赖树——正是注释里明确要避免的情况。
集中到 `[workspace.dependencies]` 属防漂移收益，不追求减行数。

### 3.4 传递依赖版本重复（`cargo tree -d`）

**2026-09-10 复测与收拢结果**：先分清"假重复"（同名同版本出现两次 = build-dep 图与 normal 图各一份，非版本分叉）：
`serde`/`serde_core`/`serde_json`/`log`/`time`/`regex-automata`/`getrandom 0.4` 属此类，**不需处理**。真分叉如下：

| crate | 初始 | 收拢后 | 归属（`cargo tree -i` 实测） | 处置 |
|---|---|---|---|---|
| `windows-sys` | 0.48/0.52/0.59/0.61 | **仅 0.61.2** | 0.48←`dirs`+`winreg 0.52`；0.52←libscoop 直依赖；0.59←`console 0.15`←`indicatif 0.17` | ✅ 已收：直依赖升 0.61 + `winreg` 0.56 + 删 `dirs`（`f9e6859`），再升 `indicatif` 0.18 → `console 0.16`（`c3858e6`）⇒ 连带 `windows-targets 0.52`、`windows_x86_64_msvc 0.52` 一起消失 |
| `hashbrown` | 0.14/0.17 | **0.14/0.17（未收）** | 0.14←`hashlink 0.9`←`rusqlite 0.31`；0.17←`indexmap` | ❌ 曾用 `rusqlite` 0.40 → `hashlink 0.12` 收拢成功，但**该提交已从历史移除**：host 图仅 −2 crate（`ahash`/`version_check`），代价是 bundled SQLite 3.45→3.53.2 大版本跳变 + lock 为 wasm 目标多出 3 条（`sqlite-wasm-rs`/`rsqlite-vfs`/`hashbrown 0.16.1`），按"收益必须可度量"口径不值 |
| `itertools` | 0.10/0.11 | 0.11/0.13 | 0.10←`criterion 0.5`（dev）；0.11←`rust-i18n-support` | ✅ 部分收：`criterion` 0.8 → 0.13（`1590efe`），0.10 消失；0.11 与 0.13 并存，上游钉死 |
| `bitflags` | 1.3.2/2.13.1 | 不变 | 1.3.2←`globwalk 0.8.1`←`rust-i18n-support`（**rust-i18n 最新 4.2.2 仍是 `globwalk ^0.8.1`**）+ `shortcuts-rs 1.1.1`（已最新） | ❌ 收不拢，只能等上游 |
| `getrandom` | 0.2/0.4 | 不变 | 0.2←`ring 0.17.14`（rustls 链；最新 ring 仍 `^0.2`） | ❌ 收不拢 |
| `syn` | 2/3 | 不变 | syn 3 已被 8 个 proc-macro 采用；syn 2 仍由 `pest_generator`/`scroll_derive`/`tracing-attributes`/`zerocopy-derive`/`yoke-derive`/`synstructure` 等钉住 | ❌ 收不拢（生态面太广） |

**净效果**：依赖树（含 dev）304 → **302**，其中 `indicatif` 一步减 3 个 crate（release 图，连带 windows 绑定 crate），`criterion` 属 dev-only（去 `itertools 0.10`、净 +1 crate：`hermit-abi`/`is-terminal` 出、`alloca`/`page_size` 入）；当前 release 图（normal+build）**280** 个 crate。
收益为编译期（少编译 windows 绑定等 crate），无功能收益。


## 4. P3 — 拆分与构建优化（先证明问题再动）

### 4.1 超长函数（36 个 ≥120 行）

| 行数 | 函数 | 位置 |
|---:|---|---|
| 378 | `execute` | `package/checkver.rs:147` |
| 376 | `commit_one_install` | `package/sync_install.rs:537` |
| 335 | `apply_autoupdate` | `package/checkver.rs:756` |
| 333 | `install` | `package/sync_install.rs:88` |
| 323 | `commit_one_remove` | `package/sync_remove.rs:179` |
| 320 | `run_auto_pr` | `package/auto_pr.rs:80` |
| 274 | `download_one_package` | `package/download.rs:120` |
| 267 | `handle` | `src/scoop_handler.rs:22` |

优先级：`checkver.rs`（1,602 行，`execute` + `apply_autoupdate` 占 713 行）拆分收益最高；
`commit_one_install` / `commit_one_remove` 结构对称，**先核对是否真能抽公共提交管线**，避免用开关掩盖 install/remove 的语义差异（参见 `no-patchwork-fixes`）。

### 4.2 构建/体积

- 现状：`opt-level = "z"` + `lto = "fat"` + `codegen-units = 1` + `strip` + `panic = "abort"`，`hok.exe` ≈ 8.9 MB。
- 代价实测：本次 `cargo bloat --release` 重建 40 分钟仍未完成（卡在 `git2`/`rusqlite` 的 C 编译 + 胖 LTO），已中止。
- 可选折中：`opt-level = "s"`（体积接近 z、性能明显更好）、`lto = "thin"`（release 构建时间大幅下降）。
  若"体积最小"不是硬指标，建议至少评估一次；否则保持现状但给开发用 profile（如 `[profile.dev-fast]`）留出快路径。

## 5. 明确不建议动

- `cmd/*.rs` 之间的文本重复（1.55%）多为 clap 派生样板与测试夹具；主 crate AST 克隆仅 10 组。
- `crates/libscoop/src/shim.rs` 与 `crates/hok-shim` 并存：双变体（生成器 vs shim 本体）是刻意设计。
- libscoop 30% 测试代码占比高——那是测试，不是重复。
- `download.rs` / `download_fragmented.rs` / `download_apps.rs` 分工清晰（`download_fragmented` 被 `download_apps` 使用），不是重复实现。

## 6. 仓库级杂项

- 未跟踪文件：`AGENTS.md`、`.agents/docs/`、`reasonix.toml`、`.agents/docs/active/audit/scoop-gap-analysis-2026-08.md`、`scripts/analyze_cohesion.py`、`scripts/analyze_rust_project.py`。
  `.gitignore` 已忽略 `.reasonix`、`.dsh`、`scripts/benchmark-output/`、`*_report.txt` 等；上述未跟踪项需要一次"提交或忽略"的决断，否则会持续出现在 `git status`。
- `scripts/` 下分析脚本（`analyze_rust_project.py`、`analyze_cohesion.py`、`check_helpers.py`、`extract_cli_help.py`）与本次新增的 `.agents/tmp/*.py` 功能有重叠，建议收敛到一处（本次审计脚本可并入 `scripts/`）。

## 7. 建议执行顺序

1. P1 全部（未用依赖 / 死键 / clippy --fix / 移除无用 allow）——一个 PR 即可，零风险。
2. 3.3 `[workspace.dependencies]` + 3.2 clap 参数组——防漂移，机械改动。
3. 3.1 innospect patch 上游化（周期长，可并行推进）。
4. 4.1 `checkver.rs` 拆分 / 4.2 profile 评估——需要先有明确问题陈述（性能或维护痛点）。

## 8. 执行结果（2026-09-10）

| 项 | 状态 | 提交 | 说明 |
|---|---|---|---|
| P1.1 未用依赖 | ✅ | `b1cfdc7` | 删 `proc-macro2`（hok-i18n-derive）、`goblin`（innospect）；**`encoding_rs` 保留**——`codepage` 以 `default-features = false` 引入它，去掉直接依赖会失去 `alloc` 门控的 `decode*` 方法（已加注释说明） |
| P1.2 i18n 死键 | ✅ | `cd88f02` | en/zh 各删 18 键，键集保持一致；**"zh 缺 3 键"是误报**（见 §2.2 更正） |
| P1.3 clippy | ✅ | `8522a48` | 工作区 crate 全部清零；`innospect` 3 条 `chunks_exact_to_as_chunks` 故意不改（保持 fork delta 最小） |
| P1.4 constant.rs | ✅ | `4ccb285` | 移除模块级 allow 后暴露：`REGEX_ARCHIVE_ZSTD` 无用户→删除；`BUCKET_PRIORITY`（+`BUCKET_PRIORITY_ORDER`）仅被 bucket 单测使用→改定点 allow + 注释 |
| P2-7 workspace deps | ✅ | `37222e0` | 13 个跨 manifest 依赖收敛到 `[workspace.dependencies]`；**Cargo.lock 零 diff**（版本/feature 完全等价） |
| P2-8 windows 依赖 | ✅ | `f9e6859` | `windows-sys` 4 版本 → 2（0.59/0.61）；删 `dirs`；`winreg` 0.52→0.56（`RegKey` 不再 `Sync`，改为按需 `RegKey::predef`，无额外注册表访问）；`internal/path.rs` 新增 `home_dir`/`config_dir` 为 Windows-first 链式回退（无 `cfg` 分支，用户 2026-09-10 定） |
| P2-6 clap 参数组 | ❌ 否决 | — | 先按「声明完全相同的 3 个 flag（`--offline`/`--no-hash-check`/`--assume-yes`）做 flatten」实现并验证（37 个命令 `--help` 逐字节一致），随后**回退**：净 **+54 行**（+118/−64）、新增 15 处 flatten + 14 处 `.enabled()` 间接层，收益仅为 3 个从未漂移的 flag 单点化；且 `--global`（9 处/6 种文案）、`package`（8 处/8 种文案）、`escape_hold`（4 处/2 种文案）、`ignore_failure`（`update` 无 `-f`）**无法**用 flatten 合并（会统一掉各命令定制帮助文案/flag 集），导致「3 个用 flatten、其余 10 个用旧写法」的模式不一致，比改动前更差 |
| P2-6 备选路线 | ⏸ | — | 若将来有漂移证据再动：① 接受统一 flag 集，flatten 整个 `SyncArgs`（CLI 表面变化）；② `macro_rules!` 只共享声明文本（不引入 `.enabled()`，但仍只在有漂移时才值得） |
| 3.1 innospect 上游化 | ⏸ | — | 未动；fork delta 仍为 2 个文件（本次未扩大） |
| 3.4 传递依赖收拢 | ✅ 部分 | `c3858e6`/`1590efe` | `indicatif` 0.18（去 `windows-sys 0.59` + `windows-targets 0.52` + `windows_x86_64_msvc 0.52`，windows-sys 收敛为单版本 0.61.2）、`criterion` 0.8（dev，去 `itertools 0.10`，并改用 `std::hint::black_box`）；树 304→302（release 图 280）。**`rusqlite` 0.31→0.40 曾实现并验证通过，但已从历史移除**（host 仅 −2 crate，代价是 SQLite 3.45→3.53.2 与 lock 变杂）。`bitflags 1`/`getrandom 0.2`/`syn 2` 上游钉死，收不拢（详见 §3.4） |
| 4.1/4.2 拆分与 profile | ⏸ | — | 未动 |
| **TLS 后端换 schannel** | ✅ | `4fc01f9` | reqwest `rustls-tls` → `native-tls`（Windows=schannel，系统证书库/代理，与官方 Scoop 一致）。去 10 个 rustls 链 crate（含 `ring`）+ 消掉 `getrandom 0.2` 分叉，新增 4 个；树 302→297（release 图 280→275）。零业务代码改动；新增 `internal::network::tls_https_smoke`（`#[ignore]`）在切换前后各验一次真实 HTTPS |
| **git2 关 ssh feature** | ⏸ 已分析未做 | — | 官方 `lib/buckets.ps1` 明确支持 ssh 形态（注释 "Support ssh and http syntax"、URL 校验引用 git-clone 文档），但 **hok 的 libgit2 凭据回调缺 `SSH_KEY` 分支、也没有 hostkey 校验回调** → 实测报 `invalid or unknown remote ssh hostkey`，**ssh 从来就不可用**。若做：① 关 feature 去掉 `libssh2-sys`（C 构建）+ 在 `bucket add` 对 ssh 形态显式报错 + 登记 gap；② 或补齐 ssh（保留 libssh2-sys，且 libgit2 不读 `~/.ssh/config`，仍不等同系统 git） |
| **zstd → ruzstd** | ❌ 撤回 | — | 实测否决：ruzstd 的 `StreamingDecoder` **只解第一帧**（两帧拼接实测 4096/8192，静默截断），现状 C zstd 正常跨帧；且收益被复测修正为仅 ~10.6 s 冷构建 + 31.5 MiB 产物（早先 43.8 s/160 MiB 系脏状态误测）。生态替代品盘点：纯 Rust 仅 `ruzstd` 成熟，其余 `oxiarc-zstd`/`structured-zstd`/`zstd-pure-rs`/`rust-zstd`/`zstd-complete` 均 0.0.x–0.x 低采用度 |
| **rust-i18n 自研替换** | ✅ | `c99f07e` `cb34afb` `84fe376` `c0dd4a1` | 已落地（2026-09-11），详见下方 §9。发布图 **265 → 240（−25）**：移除 26 crate、新增 `hok-i18n` 自身；`-e normal,build` 272 → 250（build-dep 新增 yaml-rust2/arraydeque/foldhash）。`itertools 0.11` 分叉、`serde_yaml`、`unsafe-libyaml` 全部消失。提示词：`.agents/docs/reports/prompts/i18n-inhouse-replacement-prompt.md` |

历史操作备注：`refactor/edition-2024` 全分支未推送（无 upstream），因此 2026-09-10 用 `git rebase --onto c3858e6 15b09be` 直接删除了 rusqlite 提交，criterion 提交被重放（`a00e726` → `1590efe`，已重新 GPG 签名）；备份分支 `backup/pre-rusqlite-drop` 保留在本地。

验证口径：每个主题提交前跑 `cargo check --workspace --all-targets` + `cargo clippy`；P1.1/P1.3+P1.4/P2-8/依赖升级各跑一次 `cargo test --workspace`（515 passed；`internal::git::tests` 在全局 `core.autocrlf=true` 下偶发失败，已在 HEAD 复现确认与本次改动无关），criterion 升级另跑 `cargo bench --no-run`；重写历史后追加 `cargo check --locked` 校验 lock 自洽。

## 9. 第 4 项执行结果：rust-i18n → 自研 hok-i18n（2026-09-11）

### 9.1 提交

| 提交 | 主题 |
|---|---|
| `c99f07e` | `feat(i18n)`：新增 `crates/hok-i18n`（build.rs 生成语言表 + 运行时 `t!`/`set_locale`）；顺带修 `cmd.header_buckets` 重复键 |
| `cb34afb` | `test(i18n)`：旧实现渲染基准 `tests/data/i18n_reference.tsv` + 对照测试 `tests/i18n_parity.rs` |
| `84fe376` | `refactor(i18n)`：269 处调用点切换（CLI 229 / libscoop 26 / derive 14） |
| `c0dd4a1` | `chore(deps)`：删 rust-i18n + Cargo.lock |

提示词建议拆 4 个提交（新 crate / CLI / libscoop+derive / 删依赖），实际按「新 crate / 基准 / 全量切换 / 删依赖」拆。原因：**derive 与 CLI crate 无法分开切换**——derive 生成的代码在调用方展开，旧写法依赖 `rust_i18n::i18n!` 在 CLI crate 生成的 `crate::_rust_i18n_t`，去掉 `i18n!` 就得同时改 derive，否则中间提交不编译。

### 9.2 验收（4 条全过）

1. **逐键渲染对照**：`i18n_reference.tsv` 记录旧实现 **458 键 × 6 locale × 2 列 = 2748 条**渲染；`cargo test -p hok --test i18n_parity -- --include-ignored` 逐条比对通过（"2748 recorded renderings reproduced exactly"，0 mismatch）。
   多出的 4 个 locale 是**实测语义**而非推测：`en-US`→en、`zh-CN`→zh（子标签截断），`fr`/`""`→键本身。
   ⚠️ 提示词 §2.5 写的「当前语言 → en → 键本身」**与实测不符**：旧实现只调用 `i18n!("locales")`，从未配置 `fallback =`，`_RUST_I18N_FALLBACK_LOCALE` 恒为 `None`，因此**没有 en 兜底**（`en`/`zh` 键集由 `tests/i18n_keys.rs` 保证齐平，所以此前不可观测）。新实现按实测行为实现，未加 en 兜底。
2. **CLI 帮助逐字节一致**：63 个 `--help` 面（36 命令 + 嵌套子命令 + `help` 子命令，en/zh 各一份，一次性采样）before/after **原始字节比对 0 差异**（连 `built <时间戳>` 都相同——build.rs 未重跑，比预期更严格，未做任何归一化）。
3. **`tests/i18n_keys.rs`** 适配后通过（宏名仍是 `t!`，扫描器无需改动）；全仓 515 lib 测试通过。
4. **依赖图**：`-e normal` **265 → 240**（移除提示词列的 26 个，新增 `hok-i18n` 自身）；`-e normal,build` 272 → 250。`itertools 0.11`、`serde_yaml`、`unsafe-libyaml` 全部消失（`itertools` 仅剩 dev/criterion 的 0.13）。

### 9.3 与提示词的偏离（均已在提交说明中写明理由）

| 项 | 提示词 | 实际 | 理由 |
|---|---|---|---|
| 回退链 | 当前语言 → en → 键 | 当前语言子标签链 → 键（**无 en 兜底**） | 实测旧实现无 en 兜底；见 9.2.1 |
| YAML 解析 | 自研或 build-dep 任选 | build-dep `yaml-rust2` 0.12（纯 Rust、无 unsafe） | 区域文件含 `[ls]` 流式序列等 YAML 特性，手写解析器需复刻 serde_yaml 的标量/集合解析才等价；build-dep 不进发布图 |
| 新 crate 位置/职责 | 建议 libscoop 的 build.rs 生成、hok-i18n 只消费 | hok-i18n 自带 build.rs | `hok-i18n` 是依赖图里的**单点**，表只生成一次也只内嵌一次，不会出现「第三份内嵌」；且无需让 libscoop 承担生成职责 |
| 提交拆分 | 4 个（新 crate / CLI / libscoop+derive / 删依赖） | 4 个（新 crate / 基准 / 全量切换 / 删依赖） | derive 与 CLI crate 无法分开切换；见 9.1 |

### 9.4 顺带发现

- **`cmd.header_buckets` 重复键已修**（`locales/{en,zh}.yml` 各删 1 行，两处值完全相同 ⇒ 零行为变化）。旧 serde_yaml 静默取最后一个，新解析器 `yaml-rust2` 直接报错——这类重复键以后会在**编译期**失败，是有意保留的严格性。
- **【已交接，未修】clap 参数解析错位：`hok checkver` / `hok reset`** —— 独立议题，执行提示词 `.agents/docs/reports/prompts/clap-arg-parsing-corruption-prompt.md`。

  症状（实测）：全部 **50 个帮助面 × {en,zh} × {`--help`,`-h`} = 200 次探测中恰好 8 个坏**，全部落在 `checkver` 与 `reset`；且**不止帮助**，功能也坏：

  ```
  $ hok reset -a            → error: a value is required for '--lang <LANGUAGE>' …   # reset 全部应用不可用
  $ hok checkver --version abc  → error: invalid value 'abc' for '--timeout <TIMEOUT>'  # 用错了 value_parser
  $ hok checkver --lang abc     → ERROR 错误: 'abc' 不是目录                              # 被当成 --dir 吃掉
  ```

  根因（已读 clap 源码确认）：`src/cmd/mod.rs::localize_structural_headings` 先 `cmd.build()`（**递归**，把全部子命令一并标记为 `AppSettings::Built`），之后 `localize_heading_level` 才用 `Command::mut_arg` 批量改参数；而 clap 4 的 `mut_arg` = `MKeyMap::remove_by_name` + `push`，**两个操作都不维护 `MKeyMap.keys` 里缓存的「参数名 → 下标」**，`_build_self` 又因 `Built` 标记不会重跑（第二次 `build()` 是 no-op）。同序「删+推」是一次整体旋转、结束时恰好复原，所以 49 个命令侥幸正常；真正造成位移的是 `for special in ["help","version"]` 只看 id 不看 action，把 checkver/reset **用户自己定义的 `version` 参数**（全仓仅此两处）挪到了末尾，于是 `--help` 查到的是被挪过来的那个参数。同一段代码还把 clap 自动 `--version` 的文案（"Print version"）套到了 checkver 自己的 `--version` 上。

  与本次 i18n 改动无关：`target/i18n-verify/before/`（改动前的二进制、`4fc01f9`）里 `checkver.txt`/`reset.txt` 已是同样的报错，改动前后 dump 逐字节一致；`git diff 4fc01f9..HEAD -- src/cmd/mod.rs` 除改名外只有 `-use rust_i18n::t;` 一行，`localize_*` 逻辑一字未动。

  当时未修的原因：修复会把 `--help` 从「报错」变成「正常帮助」，污染本次「用户可见文案零变化」的验收口径。**该缺陷比本报告的其余剩余项都更严重（有功能性不可用），建议优先处理。**

## 10. 依赖归并 A1：当前版本解析单入口（2026-09-12）

提示词：`.agents/docs/reports/prompts/current-version-resolution-unification-prompt.md`。**先做官方对照实验，再落地**；下面是固化在 `install_state.rs` 测试注释里的官方 ground truth。

### 10.1 官方对照表（Windows PowerShell 5.1 实跑 `Select-CurrentVersion` / `Get-InstalledVersion`）

夹具 = 临时 `apps/app/`，dot-source 官方 `lib/versions.ps1`，覆写 `get_config`/`appdir`/`parse_json`：

| 夹具 | NO_JUNCTION | 官方 `Select-CurrentVersion` | `Get-InstalledVersion` |
|---|---|---|---|
| `NJ5-two-versions`（1.0.0/2.0.0，mtime 递增） | true | `2.0.0` | `1.0.0,2.0.0` |
| `NJ5b-tie-mtime`（两版 mtime 相同） | true | `1.0.0` | `2.0.0,1.0.0` |
| `single-version` | true | `3.0.0` | `3.0.0` |
| `NJ4-stale-current`（陈旧 `current/manifest.json`=`0.9.9` + 1.0.0/2.0.0） | true | **`2.0.0`（忽略陈旧 current）** | `1.0.0,2.0.0` |
| `NJ3-current-nightly`（current manifest=nightly，无链接目标） | true | `1.0.0-nightly`（走 mtime 兜底） | `1.0.0-nightly` |
| `J1-junction-normal` | false | `1.2.3`（manifest） | `1.0.0,1.2.3` |
| `J2-junction-nightly` | false | `1.2.3-nightly`（链接目标目录名） | `1.2.3-nightly` |
| `J3-junction-no-manifest`（有 junction 无 manifest） | false | `2.0.0`（mtime 兜底） | `1.0.0,2.0.0` |
| `J4-junction-garbage`（manifest 非法 JSON） | false | `2.0.0`（warn + 兜底） | `1.0.0,2.0.0` |
| `OLD-filter`（`1.0.0`/`_1.0.0.old`/`_.old`/`_fooold`/`_foo.old.bak`） | true | `_fooold` | `1.0.0,_fooold` |

### 10.2 复评结论：合并成立，但提示词三处设计需修正

**上游原意澄清（提示词误读之处）**：`NO_JUNCTION` 开关在官方是**函数内**分支（`versions.ps1:53` 的 `if (!(get_config NO_JUNCTION))`），而**调用方一律无条件调用** `Select-CurrentVersion` —— `cleanup`(`scoop-cleanup.ps1:31`)、`list`(:30)、`uninstall`(:38/:50)、`hold`/`unhold`/`reset`/`info`/`update`(:262)、`installed`/`app_status`(core.ps1:394/549)、`currentdir` 全部如此。所以提示词把「hok 无条件读 `current\manifest.json`」列为疑点 a 是对的，但根因不是「调用方不该无条件调」，而是**hok 把那个 `if` 丢了**。真实分叉（NJ4）：hok 答 `0.9.9`，官方答 `2.0.0`。

| # | 提示词设计 | 判定 | 理由 |
|---|---|---|---|
| 1 | `CurrentVersionPolicy { Default, AllowBrokenCurrent }` 枚举 | **不加** | 该开关只服务 `query.rs` 的"坏 current → 标 install-failed"，与版本解析正交；加参数会把"为什么这里允许坏 current"变成调用方布尔。真正缺的是**模式感知**（上面那个 `if`）与**来源可分辨**。改为解析器返回 `CurrentVersion { version, source }`，`source` 让 `query.rs` 直接判定"junction 模式下走了 mtime 兜底 = 损坏"，无需预检查、无需布尔位。 |
| 2 | `Session::current_dir_name` 必须复用同一判定 | **不动** | 它不解析任何东西——`no_junction ? version : "current"` 是**目录名构造**，等价官方 `currentdir` 的非 `Select-CurrentVersion` 分支。接进解析器会把"已知版本号"重解析一遍，还可能落到别的目录。 |
| 3 | `sync_install.rs` 也改调解析器 | **不动（改走 `versioned_dir`）** | 那里拿的是**已知的旧版本号**，要的是真实版本目录；`current` 只是指向它的 junction（且 `link_current` 尚未执行）。经解析器 = 重解析已知值，且解析器在坏 manifest 时会兜底到别的目录。改为 `session.versioned_dir(name, old_version)`，消掉 `sync_install` 那处 `if no_junction` 分支本身。 |

### 10.3 落地内容

- `package/install_state.rs`：`resolve_current_version(pkg_dir, no_junction) -> Option<CurrentVersion>` 成为唯一入口；模式开关进函数内部（对齐 `versions.ps1:53`），`nightly` 链接目标与 mtime 兜底（`Get-InstalledVersion`）都在这里。`current_meta_dir()` 供"元数据在哪个目录"复用。新增 10 条对照测试（10.1 表逐行固化，含 NJ4 回归——旧行为会答 `0.9.9`）。
- 5 个调用点（+`broken_package`）收敛：`list.rs`（删 `read_link`）、`cleanup.rs`（删内联闭包 `read_link`）、`query.rs`、`sync_remove.rs`（`uninstall` + `reset` 两处，提示词漏列了 `reset`）、`install_state.rs`。全文已无第二处 `read_link(...current)`。
- `query.rs` 顺带收窄一处差异：旧代码在 `current` 元数据不全时先 `return None`（连 `--broken` 都不上报），现在按官方语义解析版本后再尝试解析元数据，解析失败才 `broken_package`。
- **顺带缺陷已修**：`package/checkup.rs` 在 `no_junction=true` 下必然误报每个包 `no 'current' symlink`（官方无对应命令，属 hok 自有诊断），现按模式检查版本目录元数据；新增 3 条测试。
- 测试夹具修正：`cleanup.rs` 的 `install_versions` 原来只建 junction 不写 `current\manifest.json`（真实安装必有），改为写入——否则解析器按官方语义走 mtime 兜底会删错版本。这正是"单一入口"暴露出的旧夹具失真。
- 新增 `Session::app_current_dir(name)`（= 官方 `currentdir` 的形态）；**删除** `Session::current_dir(name)`——它是无模式感知的 junction 路径助手，`prefix` 正是踩它才在 `no_junction` 下报"未安装"，无任何调用方后即移除，避免下一轮再踩。

**提交**：`e7c15e0`（解析器 + 消费者收敛）→ `81030c5`（checkup）→ `6fce8a3`（hold/prefix 对齐）→ `909bab4`（删 `current_dir`）。

### 10.4 有意不跟随的官方行为（登记，不修）

| 项 | 官方 | hok | 判定 |
|---|---|---|---|
| mtime 并列时取谁 | `Sort-Object` + 管道解包后**反转枚举序**（`2.0.0,1.0.0` → 取 `1.0.0`） | 稳定排序后取最后枚举项 | 同为非确定序，无法也无必要对齐；测试只断言"命中候选之一" |
| `current\manifest.json` 解析失败 | `warn "Error parsing JSON at ..."` 用户可见 | 静默走兜底 | 提示词 §6 明确"要不要 warn 单独立项"，本次不夹带 |
| `_*.old*` 过滤大小写 | `-notlike` 大小写不敏感（`_foo.OLD` 也过滤） | `contains(".old")` 大小写敏感 | 实测 5 个边界名结果一致；Windows 上 `.old` 由 Scoop 自身生成，恒为小写 |
| `currentdir` 解析失败 | 拼出 `apps\<app>\` 这种半截路径 | 用已知版本号 | 官方路径本身是缺陷，不复制 |

### 10.5 「当前版本」消费者的逐命令对标（2026-09-12 追加）

单入口解决的是「怎么解析」；这一节核对每个调用方**是否都以官方的方式用这个结果**。官方对照源：`lib/core.ps1:355` (`currentdir`)、`libexec/scoop-{cleanup,list,unhold,reset,info,update}.ps1`、`bin/uninstall.ps1`、`core.ps1` 的 `installed`/`installed_apps`。

| 命令 | 官方取版本的方式 | hok | 判定 |
|---|---|---|---|
| `cleanup` | `Select-CurrentVersion`（scoop-cleanup.ps1:31），据此删其余版本目录 | 解析器 | ✅ 已对齐 |
| `list` | `Select-CurrentVersion`（scoop-list.ps1:30），再 `versiondir $app $ver` 读 install.json | `query_installed` → 解析器 | ✅ 已对齐 |
| `uninstall` | `Select-CurrentVersion`（bin/uninstall.ps1:38）→ `versiondir`/`installed_manifest`/`install_info`；`unlink_current` 后用其返回值做 `env_rm_path` | `resolve_current_version` → `version_dir`；`unlink_current` 后 `psmodule/env` 各自解析 | ✅ 已对齐（删目录/`current` 收尾也与 scoop-uninstall/cleanup 一致） |
| `hold` / `unhold` | `NO_JUNCTION ? Select-CurrentVersion : 'current'`（scoop-hold.ps1:49-55 / unhold:49-55）写 `install.json` | **原硬编码 `current`**（`hold.rs:44`） | ❌→✅ **本次修**（`Session::app_current_dir`） |
| `reset` | 显式版本优先，否则 `Select-CurrentVersion`（scoop-reset.ps1:48-50）→ `installed_manifest`+`link_current` | `resolve_reset_target`：显式版本 / 解析器；manifest 从**版本目录**读（对齐 `installed_manifest`）、install.json 可选 | ✅ 已对齐 |
| `info`（版本） | `app_status` → `Select-CurrentVersion`（core.ps1:549） | `query_installed` → 解析器 | ✅ 已对齐 |
| `info -v`（目录） | `currentdir $app $global` + `versiondir $app $manifest.version $global`（scoop-info.ps1:53-59） | **hok 的 `info` 没有 `-v`**，不报这两个路径 | ⚠️ 未对齐（能力缺口，非解析错位；登记见 `scoop-gap-audit`） |
| `update` | `Select-CurrentVersion` 取旧版本（scoop-update.ps1:262） | query 驱动（`query_installed` 解析器取 installed_version），**恰好一致** | ✅ 已对齐 |
| `installed`（core.ps1:394） | `$null -ne (Select-CurrentVersion …)` | `load_install_state`：解析器解析不出即 `NotInstalled` | ✅ 已对齐 |
| `installed_apps`（core.ps1:396） | 只枚举目录、不判 installed | `query_installed` 解析不出即跳过（与 `installed` 等价） | ✅ 一致（比官方更严：空目录不当已安装） |
| `currentdir`（core.ps1:355） | `NO_JUNCTION ? appdir\Select-CurrentVersion : appdir\current` | `Session::app_current_dir`（本次新增；原 `current_dir` 已删） | ✅ 已对齐 |
| `prefix`（非清单内） | `currentdir`（scoop-prefix.ps1:12-14） | **原 `session.current_dir()` 硬拼 `current`** | ❌→✅ **本次修** |
| `status`（非清单内） | `app_status`（core.ps1:549） | `query_installed` → 解析器 | ✅ 已对齐 |

**结论**：清单 9 项里 8 项本就对齐，`hold`/`unhold` 未对齐（本次修）；顺带查出清单外的 `prefix` 同样未对齐（本次修）；`info -v` 是 hok 缺能力（不修本议题）。

顺手修正的第三处差异：`currentdir` 在解析失败时官方会拼出 `"$appdir\"` 这种半截路径（`Select-CurrentVersion` 返回 `$null` 也照拼），hok 的 `app_current_dir` 返回 `None` 让调用方报"损坏安装"——属有意偏离，已写入代码注释。

### 10.6 验收

`cargo test -p libscoop` **529 passed**（基线 515；净新增 14 条业务测试——`install_state` 对照 10 条、`checkup` 3 条、`hold` 2 条、`cleanup` 回归 1 条、`session::app_current_dir` 1 条，另有 3 条 `sync_remove` 既有测试随签名更名）、`cargo test --workspace` 全绿（`permission::tests::grant_users_write_inherit_{sets_dacl,is_idempotent}` 在本沙箱环境基线即失败，与本次无关）、`cargo clippy --workspace --all-targets` 仅剩 vendored `innospect` 既有 4 条（含 `lib test` 那条 `unused import: sha1::Digest`，亦在 vendored 内）。全文无第二处 `read_link(...current)`；`join("current")` 仅剩解析器内部（对齐 `versions.ps1:53` 的读取点）、`checkup` 的 junction 存在性检查、`app_current_dir` 的 junction 分支与 `link/unlink_current` 本身。

## 11. 依赖归并 A2：宽容 JSON 解析单入口（2026-09-12）

提示词：`.agents/docs/reports/prompts/tolerant-json-single-entry-prompt.md`。照例**先复评再落地**：4 处实现 / 3 种策略的现状与官方调用点表均实测确认，但提示词有 6 处口径需修正（11.2 表中 ❌ 项），另有 2 处过时信息。

### 11.1 官方对照（本地 `../Scoop`）

官方只有一个解析实现（宿主的 `ConvertFrom-Json`），差异全在调用点错误策略上；hok 的对应物是库函数，所以「实现唯一 + 策略显式」正是官方形态的映射：

| 官方调用点 | 官方错误策略 | hok |
|---|---|---|
| `lib/manifest.ps1:5-12` `parse_json` | warn + `$null` | `internal::json::parse_str` → `Err`（有意偏离，见 11.4） |
| `lib/manifest.ps1:14-32` `url_manifest` | warn + `$null` | → `Err` |
| `lib/core.ps1:82-95` `load_cfg` | `Write-Host`，不给对象 | → `Err`（`Config::load` 忽略后回落 "config not found"） |
| `lib/buckets.ps1:40` / `lib/json.ps1:130` | 抛 | → `Err` |
| `libexec/scoop-search.ps1:96` | `-ErrorAction Continue` | 无对应 |
| `lib/database.ps1:175` | `-ErrorAction SilentlyContinue` | 无对应（hok 唯一吞错点 = `parse_str_or_none`，仅 formatjson 诊断） |
| `bin/checkhashes.ps1:65`、`bin/auto-pr.ps1:190`、`libexec/scoop-import.ps1:19`、`Invoke-AutoUpdate`（已解析的 `$manifest`） | `parse_json` | 本次补：原先用裸 `serde_json`（11.3） |

### 11.2 复评结论：合并成立，提示词 6 处需修正

| # | 提示词 | 判定 | 依据 |
|---|---|---|---|
| 1 | §2.1 结论「hok 的 json5 回退是官方 PS 5.1 的**有意超集**」 | ❌ 错 | 2026-09-12 实测（`powershell.exe` 5.1.26100 / `pwsh` 7.6.5，脚本 `.agents/tmp/json-tolerance-probe.ps1`）：`{"a":01}` **两宿主都接受**，而 json5（`json5-0.4.1/src/json5.pest:25` 的 `decimal_integer_literal`）与 serde_json 都拒绝 → hok 在该用例上比官方更严。关系是「不同集合」而非超集 |
| 2 | §2.1 行注释行标注「用例构造不当，请重测」 | ✅ 定论 | `{"a":1 // c}`（无换行）**两宿主都 FAIL**（注释吃掉 `}`）；带换行 PS 7.6.5 OK、PS 5.1 FAIL |
| 3 | §5.2 要求测试断言「hok 宽容度是有意超集」 | ❌ 写不出 | 见 #1；改为逐用例裁决表，宿主列仅作记录（宿主行为非 hermetic，不进 CI 断言） |
| 4 | §6.2 `grep trim_start_matches` 全仓只剩一处 | ❌ 不可达 | 另有 5 处非 BOM 用途（archive/checkhashes/checkver/version/sync_install）；验收须限定 `'\u{FEFF}'` |
| 5 | §3 策略枚举 `{Strict, Tolerant, Silent}` | ❌ 拆轴错 | 合并后 4 个点全是 Tolerant/Silent，`Strict` 成死变体（正是 §3 自己禁的「开关掩盖语义」）；改为解析链固定 + 唯一吞错入口 `parse_str_or_none` |
| 6 | §3 `parse_file_tolerant` 统一 I/O | ❌ 不做 | 官方读 API 本就不同（`ReadAllLines` 探测编码 / `Get-Content -Encoding UTF8` / `ReadAllText`）；I/O 与字节解码留在调用点，模块只管解析 |
| 7 | §6.4 基线 515 passed | ⚠️ 过时 | A1 收官时已是 529（§10.6） |
| 8 | §7 「不要动 innospect（它自己有 json5）」 | ⚠️ 过时 | innospect 无 json5 依赖（仅 bitflags），该约束为空 |

### 11.3 落地内容

- 新增 `crates/libscoop/src/internal/json.rs`：`parse_str<T>`（`strip_bom` → serde_json 严格 → json5 回退，两者都失败报**严格错误**）、`parse_str_or_none<T>`（唯一吞错入口）。模块文档含官方调用点表、宽容度契约（11 条三方实测表）、以及「哪些读取有意保持严格」（远程载荷 `internal::github`/checkver 页面、机器生成状态 `install.json`/SQLite 行——JSON5 兜底会把坏生产者变成静默不同的值）。
- `internal/string.rs::strip_bom`：剥**一个**前导 BOM（官方编码探测只吃一个）；6 处 BOM 剥离全部收敛（4 处 JSON + psengine/script 的行级输出）。
- 4 处原实现替换（config / manifest / formatjson×2），`parse_manifest_spec` 直通包装删除（少一层间接）。
- **顺带查出的真缺口（一并修）**：checkhashes、checkver `apply_autoupdate`、auto_pr、import 仍用裸 `serde_json` 读**用户/bucket 文件**，而官方这四处都走 `parse_json` → BOM 或 JSON5 的手改 manifest 装得上、却在这四个命令失败（checkhashes 更别扭：宽容读通过、写回时严格失败）。已全部改走单入口，独立提交可单独 revert。
- 契约测试：`tolerance_contract`（11 条 hok 裁决；宿主列以 `_` 记录、不断言）、`strip_bom` 2 条、解析链 5 条、`import::parse_accepts_hand_edited_syntax` 1 条。
- 量化（`git diff --numstat`）：`7f4a6ae` +272/−54、`4e7a6c2` +25/−11；增量集中在模块文档与契约测试，生产代码是「删 4 套实现 + 1 层包装、加 2 个函数」。
- 提交：`7f4a6ae`（单入口 + 原 4 处 + 行级 BOM）、`4e7a6c2`（其余四处 manifest 读取）。

### 11.4 有意偏离与登记项（不修）

| 项 | 官方 | hok | 判定 |
|---|---|---|---|
| 解析失败的用户可见行为 | `parse_json` warn + `$null` → 调用方报「找不到 manifest」 | 返回解析错误（含行/列） | 有意偏离（比 Scoop 更好）：直接指出真问题；已登记在模块文档 |
| `{"a":01}` 前导零 | 两宿主都接受 | 拒绝（JSON5 与 JSON 同禁） | 已知差异，测试固化；真实 manifest 的版本号是字符串，影响面≈0 |
| 畸形容错集合 | 随宿主漂移（5.1 vs 7.6.5 五处不同） | 固定契约（注释/尾逗号/无引号键更宽） | hok 自定契约，测试钉死 |
| config 解析失败文案 | `ERROR loading <file>: ...` | `config file not found: ...`（`Config::load` 忽略读错误后回落） | 既有差异，登记不动（提示词亦明确不夹带 warn 类改动） |
| `formatjson` 的 schema 告警 | 无对应（官方 formatjson 只有 44 行，schema 由 Pester gate 管） | hok 自有诊断，best-effort | 提示词想把它对应到 `-ErrorAction SilentlyContinue` 是错误类比，改为「hok 自有」 |

### 11.5 验收

`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets`（当时仅 vendored `innospect` 既有 4 条；2026-09-19 起该 fork 已退役）、`cargo test --workspace` 全绿；`cargo test -p libscoop` **540 passed**（基线 531，本议题净增 9 条）。`grep "json5::from_str"` 全仓仅 `internal/json.rs` 一处；生产代码再无 `trim_start_matches('\u{FEFF}')`（`strip_bom` 内除外）。原 oracle 全绿：`config::load_tolerates_bom_config`、`manifest::bom_prefixed_manifest_parses`、`manifest::json5_manifest_parses_as_fallback`、formatjson 批量/容错用例。
