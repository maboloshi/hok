# `hok download <合法本地 manifest>` 主线程栈溢出

> 2026-09-25 ｜ 提交 `6620d69`

**现象**：`hok download <local manifest>` 主线程栈溢出（exit `-1073741571` / `0xC00000FD`），`hok info` 同一文件正常，`install` 同 manifest 也崩。判别式实测：**manifest 带 `hash` 字段**才崩（无 `hash`、非法版本、路径不存在都不崩），与 `-s`/URL/进度条/包流程无关。

**定位**：eprintln 二分把崩溃点压到 `parse_file` 里的 `serde_json::from_str::<ManifestSpec>`；再用**改 PE 头 `SizeOfStackReserve` + 二分栈大小**量化。debug 构建的最小可用主线程栈：`--version`/`home`（纯框架 clap+dispatch+logger）**768 KB** → `info min.json` 960 KB → `download min.json` **1152 KB** → `download <真实形状 manifest>` **1216 KB**；默认主线程只有 **1 MiB**。分解：框架 768 + 解析 192 + download 额外 async 层 64 + `REGEX_HASH` 首次惰性编译 128（后两项由"浅栈预编译正则 / 跳过正则"两个对照实验分别证实）。**不是深递归**——2 MiB 即跑通且结果正确；release 完全不受影响，`info` 只是链更浅刚好卡住。

**修复**：`build.rs` 增加 `cargo:rustc-link-arg=/STACK:8388608`（守卫 `CARGO_CFG_TARGET_ENV=="msvc"`；reserve 是虚拟内存、按需提交，无前置开销）。**回归测试** `tests/download_stack_overflow.rs`：CLI 级、跑真实二进制 + 一次性 loopback HTTP 服务器，下载带 `hash` 的本地 manifest 并二次跑缓存命中——libscoop 单测线程栈更大，永远复现不了本崩溃。

## 证据（实测）
- **测试有效性反证**：把 `target/debug/hok.exe` 的 `SizeOfStackReserve` 改回 1 MiB，**直接跑 `target/debug/deps/<test>-<hash>.exe`**（不经过 cargo，否则会按 fingerprint 重链接覆盖掉手改）⇒ FAIL 且报 `has overflowed its stack`；改回 8 MiB ⇒ PASS（连跑 8 次稳定）。
- **免重编译的栈规模探针**：`$pe=[BitConverter]::ToInt32($b,0x3C)`，字段在 Optional Header **`+0x48`**（`+0x18` 是 `ImageBase`，默认 `0x140000000`——改错会得到"5 GB 栈"的假象）。
- **端到端三态**（node 限速服务器）：成功 exit 0 / 2 请求 / 缓存落盘；哈希失败 exit 0 并报 `Hash mismatch`、错文件被删；缓存命中 exit 0、只有 1 次 HEAD（`正在从缓存加载 …`）。1 MiB 副本在第一个请求前即崩。
- **不回归**：`install` 走同一 manifest exit 0；另用合法 zip + `bin` 的 manifest 实测解压成功并生成 `stackapp.exe` / `shim`；release 同样 8 MiB 预留且 `--version`/`info` 正常。
- 门禁：改动文件 `rustfmt --edition 2024 --check` 0 差异；`cargo clippy --workspace --all-targets` 0 警告；`cargo test --workspace` exit 0。落地：`worktree-merge.ps1 -VerifiedRef 6620d69` → `refactor/edition-2024` 线性 ff（无 merge commit）。

## 教训
1. **"崩在某函数"先分清预算不足 vs 无界递归**：放大栈能跑通就是预算问题（Windows 主线程栈来自 PE 头、运行时改不了），别急着按"深递归"改代码。
2. **量化要按命令矩阵做**：`info` 能跑不代表解析没问题；把 `--version`（纯框架）也纳入对照，才知道 768 KB 是框架基线、崩溃只是最后 400 KB 的事。
3. **对照实验要绕开构建系统**：`cargo test` 会重链接目标二进制，手改的 PE 头会被覆盖。
4. **CLI 级回归测试才有价值**：只跑真实二进制能暴露 PE 头主线程栈问题；libscoop 的 `spawn_range_server` 是 `#[cfg(test)]` 私有，集成测试自带一份极简 `TcpListener` 服务器更省事。
5. **自建测试服务器的 Windows 坑**：`accept()` 得到的 socket 继承 listener 的**非阻塞模式**，同步 handler 必须 `set_nonblocking(false)` + 设读写超时，否则请求字节未到就读成 `WouldBlock`、连接被静默关闭（首版测试因此约 1/8 概率假失败）。
6. **既有 rustfmt 偏差 vs 本次改动文件**：全仓 `cargo fmt --check` 本就有偏差，但本次改动文件仍按门禁 fmt 干净——`build.rs` 里 2 行 `use` 重排属该类，已在提交说明与报告登记为有意为之。

---
