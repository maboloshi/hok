# 上游 innospect 0.1.4 收录 PR #3，退役 vendored fork

> 2026-09-19 ｜ worktree `.worktrees/innospect-registry`，分支 `chore/innospect-registry`

**背景**：2026-08-16 提的 [innospect PR #3](https://github.com/ATRAPSLLC/innospect/pull/3)（lzma-rust2 + flate2/zlib-rs）已随 **0.1.4（2026-09-18）** 合并（changelog 记 "Contributed by @maboloshi"）；上游另把 **LZMA2 字典大小改为读 Inno 的属性字节**（我们 fork 里是固定 32 MiB），并清掉了新 stable 的 `chunks_exact_to_as_chunks` clippy lint。

**改动**（同 nsis 先例 `9766e7b`）：`libscoop` 的 `innospect = { path = "../innospect" }` → `innospect = "0.1.4"`（registry）；workspace `members` 去掉 `crates/innospect`；`git rm -r crates/innospect`（55 个跟踪文件、约 12.8k 行）。`Cargo.lock` 随上游 minimum 上抬：bitflags 2.13.2、chacha20 0.10.2、codepage 0.1.3、encoding_rs 0.8.41、flate2 1.1.10、pascalscript 0.1.3、miniz_oxide 0.9.1，新增 core_detect/multiversion*/simdutf8，`lzma-rs` + `crc`/`crc-catalog`（innospect 的 dev-dep）出树。

**取舍（登记）**：① innospect 自带的 79 个测试不再随本仓 workspace 跑（上游 CI 负责）；② 上游 manifest 声明的 `goblin`/`encoding_rs` 回到构建图（本地 fork 曾按 `b1cfdc7` 删掉这两个未用的 direct dep）；③ 好处是少维护 12.8k 行 fork，且 clippy 门禁不再需要 `--exclude innospect`/`--no-deps`（当前门禁命令见 skill `hok-rust-gate-practice`）。

**验证（A/B 同输入）**：临时 crate 分别用旧 fork（path）与 registry 0.1.4 解析并解码真实 Inno 安装器 `zed#1.20.2`（Inno 6.4.3，85 MB，就在 Scoop 缓存里）——**输出逐字节一致**：`files=10 bytes=392431449 digest=263bb32c…cd4e`，耗时 13s → 12s。workspace 门禁：`cargo check` / `cargo clippy --workspace --all-targets -- -D warnings` 干净；`cargo test --workspace` **750 passed / 2 failed**（仅既有沙箱 ACL `permission::tests::grant_users_write_inherit_*` 的 error 5）。

**环境坑（更正本节下方 2026-09-13 条目里的说法）**：那里记的"本机 schannel 不可用"**不准确**——`workspace-write` 沙箱下 cargo/curl 拉 crates.io 报 `schannel: AcquireCredentialsHandle failed: SEC_E_NO_CREDENTIALS`，但同一条 `curl` 在 `danger-full-access` 下返回 **200** ⇒ 是**沙箱挡住了 crypto store**，不是机器缺 TLS。要拉新的 crate 版本必须先放宽沙箱跑一次 `cargo fetch`（缓存落盘后普通构建回到沙箱内即可）；Node（自带 CA）一直不受影响。
