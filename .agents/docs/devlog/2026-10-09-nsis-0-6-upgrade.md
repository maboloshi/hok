# nsis 升到 0.6：0.5.0 的 breaking API 与 hok 无关，真正的理由是 LZMA 字典上限

> 2026-10-09 ｜ 提交 `8a93027` ｜ worktree 主树 ｜ 分支 `refactor/edition-2024`

## 结论

N1 的前提（「上游 0.5.0 含 breaking API，所以升级要评估」）**方向对但归因错**。
0.5.0 的 breaking 面全在反汇编 / 操作码分析接口：`ParamType`、`OpcodeInfo` /
`ParamLayout`、`param_layout()`、`normalize_park_opcode()`、`OPCODES` —— hok 一个都没引用。
源码级比对（0.4.0 vs 0.5.0）显示 hok 实际消费的 `strings/mod.rs`、`installer/files.rs`、
`decompress/mod.rs`、`error.rs`、`lib.rs` 的规范化 `pub` 面**完全相同**，
`installer/nsisinstaller.rs` 只是**新增** `param_layout`。`dest_path()` 的 `$INSTDIR`
剥离逻辑未变（`files.rs` 的 diff 只有文档注释破折号）。**单独升 0.5.0 对 hok 零收益。**

真正值得动的是 **0.6.0**：它给 LZMA 流声明的字典大小加了上限。hok 在 512 MiB 预算内
解析**从网上下载的、不可信的**安装器，而没有上限时一个构造的安装器可以按流声明撑到 ~4 GiB，
绕过 `max_decompressed_size`。

## 踩到的坑：升 nsis 会牵出第二个 LZMA 解码器

`nsis 0.6` 与 `innospect 0.2` 都要求 `lzma-rust2 >= 0.21`；只改 workspace 的 pin 会让树里同时
存在 `lzma-rust2 0.18.1` 与 `0.21.0` —— 因为 **`sevenz-rust2 0.21.4` 钉 `^0.18`**
（`cargo tree -i lzma-rust2@0.18.1` 定位）。两份解码器 = 双份代码与链接成本，
而这正是本仓在 flate2 上刻意避免的事（`crates/libscoop/Cargo.toml` 的注释：「so miniz_oxide is
not pulled in by any of them」）。

解：**`sevenz-rust2` 0.21 → 0.23.0**，它的要求是 `^0.21`，与 nsis / innospect 收敛到单份
`lzma-rust2 0.21.0`。0.22/0.23 的 changelog 只有 lzma/zstd 版本跟涨、新增 API 与修复
（含一条「非固实 7z 多文件解压提速」），无破坏性改动；`cargo check` 与 852 条单测全绿。
顺带：lock 里 `multiversion` / `multiversion-macros` 被移除，依赖是**净减**。

## 行为差异（已记录，未处理）

0.5.0 起 `StringTable::read` 对负偏移返回 `StringSegment::LangString` 而非空，
于是 `to_install_path()` 从 0.4.0 的回退值 `file` 变成 `$(LSTR_n)`。两者都非空，
也都不是真正的语言串替换；仓库里没有已提交的 NSIS 夹具覆盖这条路径
（语料测试是 `#[ignore]` + `NSIS_SAMPLE` 门控，CI 不跑）。`$(LSTR_n)` 的字符在
Windows 文件名里合法，不会崩。**没有**为它写"钉住现状"的测试——现状本身可疑，
钉住等于把可疑行为固化；真要做替换是另一个主题。

## 教训

1. **「上游有 breaking API」不等于「我们要付迁移成本」**：先比对**我们实际引用的那部分**
   `pub` 面，再谈代价。这次迁移成本是 0 行 `.rs`。
2. **升 A 之前先 `cargo tree -d` 看重复**：间接依赖的版本区间会把"顺手升一个"变成
   "树里多一整份解码器"。判据是 `cargo tree -i <crate>@<ver>`，不是感觉。
3. 本主题的唯一风险面（NSIS / 7z 真实解压）**没有 CI 覆盖**，只有 `NSIS_SAMPLE` 门控的手工测试；
   N1 的验证因此停在 `cargo test -p libscoop` + `cargo build --workspace`，未做真实安装器对拍。
