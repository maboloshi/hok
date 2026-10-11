# 提示词：用自研 i18n 层替换 rust-i18n（优化审计第 4 项）

> **用法**：在新会话里把本文件作为首要输入（`@.agents/docs/reports/prompts/i18n-inhouse-replacement-prompt.md`）。
> 这是一份**自包含**交接：新会话没有本议题的历史上下文，全部依据写在这里。
> **前置加载的 skill**：`hok-code-organization`、`hok-rust-gate-practice`、`git-commit-practice`、`hok-proactive-review`、`token-efficient-processing`、`no-patchwork-fixes`。

---

## 0. 一句话目标

把 `rust-i18n`（3.1.5）从 hok 中彻底移除，用仓库自研的 `hok-i18n`（运行时 + 构建期生成语言表）替换，**用户可见文案零变化**，**发布依赖图减少 26 个 crate**。

## 1. 为什么值得做（已实测，勿重新论证）

`cargo tree -e normal,build --workspace` 的图：**272 → 246（−26 crate）**。移除集合（实测，逐个确认只有 rust-i18n 链使用）：

```
rust-i18n / rust-i18n-macro / rust-i18n-support
serde_yaml（已废弃） / unsafe-libyaml（libyaml 的 Rust 转录，unsafe 密集）
glob / globset / globwalk / ignore / bstr / walkdir / same-file / winapi-util
toml / toml_edit / toml_datetime / toml_write / serde_spanned / winnow
triomphe / arc-swap / base62 / siphasher / rustversion / normpath / itertools
```

关键点：
- **`itertools 0.11` 版本分叉随之消失**（剩下的 0.13 只在 dev/criterion 里）。
- 去掉 `serde_yaml`（已废弃）与 `unsafe-libyaml`。
- rust-i18n 支持 YAML/JSON/TOML 三种 locale，**我们只用 YAML**：toml 那套纯属白带。
- 现状 locales 被**内嵌两次**（hok 与 libscoop 各一次 `i18n!`），且 `rust-i18n-support/src/lib.rs:129` 在**运行时**用 `serde_yaml::from_str` 解析。

审计出处：`.agents/docs/active/audit/optimization-audit-2026-09.md`（§8「剩余项」第 4 项）。

## 2. 现状耦合面（必须原样复刻的语义）

**用到的 API 只有三个**：

| API | 位置 | 数量 |
|---|---|---|
| `rust_i18n::t!` | `src/` 229 · `crates/libscoop/src/` 27（error.rs、package/{auto_pr,checkhashes,checkup,checkver,cleanup}.rs）· `crates/hok-i18n-derive/src/` 15 · `tests/` 3 | **274**（44 个文件） |
| `rust_i18n::i18n!("…")` | `src/lib.rs:37`（`"locales"`）、`crates/libscoop/src/lib.rs:57`（`"../../locales"`） | 2 |
| `rust_i18n::set_locale(..)` | `src/i18n.rs:62` | 1 |

必须复刻的语义（**逐条验证，不要凭印象**）：

1. **缺键行为 = 返回键本身**。证据：`crates/hok-i18n-derive/src/lib.rs:212-215`
   ```rust
   let v = rust_i18n::t!(key).to_string();
   if v == key { rust_i18n::t!(fallback).to_string() } else { v }
   ```
2. **动态键**：derive 生成的代码用 `t!(#tk)`（运行时 `&str`/`String` 表达式），所以 `t!` 必须接受非字面量表达式。
3. **插值**：`%{var}`，共 148 处；变量名全是普通标识符（`name` 27、`object` 18、`e` 12、`count` 8、`ctx` 8、`path` 7、`query` 7、`total` 6…）。调用形态 `t!("key", a = expr, b = expr)`，值可能是 `String`/`&str`/数字/实现了 Display 的类型。
4. **无复数**：`locales/*.yml` 里没有任何 `_one/_other/…` 键（已确认）。
5. **回退链**：当前语言 → `en` → 键本身。（现状行为由 `set_locale` + en 作兜底保证。）
6. **locale 文件格式**：`locales/en.yml`（29,927 B）/ `locales/zh.yml`（28,860 B），2–3 层嵌套，含引号字符串、转义、以及**块标量** `|-`（例：`cli_help.shim.long_about` 是多行文本）。
7. **语言探测不动**：`src/i18n.rs` 本来就是自研的两遍探测，只调用 `set_locale`；新方案保持该文件逻辑不变，只把 `rust_i18n::set_locale` 换成自研 API。

## 3. 设计（建议，允许合理偏离但要写明理由）

```
crates/hok-i18n/            ← 新增：运行时（不依赖任何 i18n 库）
  build.rs                  ← 扫 ../../locales/*.yml，生成 OUT_DIR/locales.rs
  src/lib.rs                ← set_locale / current_locale / lookup / t! 宏 / 回退链
crates/hok-i18n-derive/     ← 既有 proc-macro，保持不变（仅把生成的 t! 换成 ::hok_i18n::t!）
```

- **数据**：build.rs 生成 `static LOCALES: &[(&str, &[(&str, &str)])]`（语言 → (键, 值) 表），与 libscoop 既有 `known_buckets.rs` 生成套路一致（`crates/libscoop/src/constant.rs:36` 有先例）。
- **两个 crate 共用一份**：`hok-i18n` 作为公共依赖，由 libscoop 与 hok 同时依赖 → 消除现在的双重内嵌。
- **`t!` 实现**：`macro_rules!` 即可（不需要 proc-macro）：
  - `t!(literal)` / `t!(expr)` 都接受；
  - 追加 `k = v` 参数做 `%{k}` 替换；
  - 实现回退链与"缺键返回键"语义。
- **YAML 解析只在 build.rs**：需要**一个 build-dependency**（`serde_yaml_ng` / `saphyr` / `yaml-rust2` 任选，优先纯 Rust、仍在维护）。build-dep 不进发布图，但会出现在 `cargo tree -e normal,build` 里，**验收时的 246 是排除 build-dep 的口径，请统一用 `cargo tree -e normal` 复核发布图**（现状 272 同样是 `-e normal,build` 口径，二者对比要口径一致）。
  - 若选择手写解析器：仓库 `tests/i18n_keys.rs` 已有纯 std 的部分解析先例，但**块标量 `|-` 与转义**必须正确处理，且要写单测覆盖。
- **删掉**：两处 `rust_i18n::i18n!`、`rust-i18n` 依赖（根 `Cargo.toml` 与 `crates/libscoop/Cargo.toml`，以及 `[workspace.dependencies]` 里的条目）。

## 4. 迁移清单

1. 新增 `crates/hok-i18n`（含 build.rs 与生成逻辑），加进 `[workspace] members`。
2. 全仓机械替换 `rust_i18n::t!` → `hok_i18n::t!`（274 处 / 44 文件）；`use rust_i18n` 之类导入一并调整。
3. `crates/hok-i18n-derive/src/lib.rs`：把生成代码模板里的 `t!(#tk)` 改成 `::hok_i18n::t!(#tk)`（注意生成代码在**调用方 crate** 展开，路径要用绝对路径 `::hok_i18n::…`）；derive 自身内部那处 `rust_i18n::t!` 也一并改。
4. 删除 2 处 `i18n!(…)`；`src/i18n.rs:62` 改为自研 `set_locale`。
5. `tests/i18n_keys.rs`：它的扫描器按 `t!("key")` **字面量**找键（见文件头注释），需适配新宏名并继续承担「键齐平 + 字面量引用有效」两项校验。
6. 移除依赖后 `cargo tree` 复核 crate 数。

## 5. 验收条件（全部必须通过，缺一不可）

1. **逐键渲染对照**：写一个（可 `#[ignore]` 的）对照测试，遍历 en/zh 全部键，用**旧实现**与**新实现**分别渲染并逐条比较，要求完全一致（含缺键返回键的行为、含插值）。旧实现的对照数据要在删除 rust-i18n **之前**采集（可临时保留一个对比测试或先落期望值文件）。
2. **CLI 帮助逐字节一致**：采集全部子命令 `--help` 输出做 before/after diff（方法见 `.agents/docs/active/audit/optimization-audit-2026-09.md` §8 中 P2-6 的做法：`hok --help` 解析子命令名后逐个 dump，唯一允许的差异是 `built <时间戳>` 那一行）。
3. `tests/i18n_keys.rs` 适配后通过（en/zh 键齐平；源码里的字面量键必须存在）。
4. **依赖图达标**：同一口径下 crate 数由 272 降到约 **246**（±build-dep）；`itertools 0.11` 与 `serde_yaml` 不再出现。

门禁（每个提交前）：`cargo check --workspace --all-targets` + `cargo clippy --workspace --all-targets`（允许只余 vendored `innospect` 的 4 条）+ `cargo test --workspace`（当前 515 passed；`internal::git::tests` 在全局 `core.autocrlf=true` 下有已知偶发失败，与本题无关）。

## 6. 陷阱（血泪点，逐条遵守）

- **不要顺手改文案**：这是纯替换，任何文案变化都会被第 1/2 条验收抓住。
- **`t!` 必须支持非字面量**（derive 生成代码用运行时 key）。
- **缺键返回键本身**，不是返回空串、不是 panic。
- **块标量**：`long_about` 这类多行 YAML 必须以 `\n` 原样保留（帮助文本里是多行）。
- **路径**：libscoop 的 locales 在仓库根 `locales/`（从 `crates/libscoop` 看是 `../../locales`），新 crate 若放在 `crates/hok-i18n` 需自行算相对路径，或让 libscoop 的 build.rs 生成、`hok-i18n` 只做运行时消费（推荐后者，避免第三份内嵌）。
- **不要动 vendored `crates/innospect`**（保持与上游 diff 最小）。
- **不要整包 `cargo fmt`**：仓库存在既有 fmt diff；只 fmt 本次改动文件（`rustfmt --edition 2024 <files>`）。
- **git 一律 `-c core.autocrlf=false`**；本机 `commit.gpgsign=true`，提交会自动签名。
- 项目口径（2026-09-10 用户明确）：**已脱离上游，`libscoop` 不再按 crates.io 发布库对待，仅 Windows**——不需要为 docs.rs/跨平台做任何保留。

## 7. 建议的提交拆分（一主题一提交）

1. `feat(i18n): add hok-i18n runtime and build-time locale tables`（新 crate + 生成 + 单测，此时尚未切换调用方）
2. `refactor(i18n): migrate the CLI crate to hok-i18n`（`src/` 229 处 + `src/i18n.rs` + `src/lib.rs`）
3. `refactor(i18n): migrate libscoop and the clap help derive`（27 处 + derive 15 处 + 删 `i18n!`）
4. `chore(deps): drop rust-i18n`（删除依赖 + `[workspace.dependencies]` 条目 + Cargo.lock）

每步跑门禁；第 1 步之后可以先只切一个文件验证机制，再批量替换。

## 8. 如果决定不做

把结论与本文件的实测数据回写到 `.agents/docs/active/audit/optimization-audit-2026-09.md` §8，并在 Hindsight 记一条更正（避免下一轮重新论证）。判定门槛是"收益可度量且显著"：本题 **−26 crate + 消除一条版本分叉 + 去掉已废弃依赖**，已达门槛；唯一代价是工作量（约一个小型会话）。
