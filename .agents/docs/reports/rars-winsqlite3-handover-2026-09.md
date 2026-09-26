# 交接：rars / winsqlite3 两项依赖替换（2026-09-22，**未完成**）

> 为什么写下这段：自动 goal 轮次（6/6）用尽、两个后台子代理仍在跑。新会话看不到原对话，换会话前必须有交接。
> 相关技能：`hok-agent-orchestration`、`hok-worktree-parallel`、`investigate-first`、`no-patchwork-fixes`。

## 分支与状态

- 主线 `refactor/edition-2024` @ **65b83ee**（线性历史，只接受 `--ff-only`，禁 merge commit）。
- 两条分支**均尚无提交**，`merge-base --is-ancestor` 校验通过：

| worktree | 分支 | 子代理 | 当前 |
|---|---|---|---|
| `.worktrees/rar-rars` | `feat/rars-swap` | `3c933879-55dc-4545-8d5f-f182df7313b3` | 未提交：`Cargo.toml`(+`rars`)、`Cargo.lock`、`tests/fixtures/rar/`(96 KB/28 文件) |
| `.worktrees/sqlite-winsqlite3` | `feat/winsqlite3` | `a1f2b786-65b0-48de-8474-2f3f51dfe0c4` | ✅ **已落地**（2026-09-22 ff 到主线，tip `eeef0a2`，见文末"落地记录"） |

两条会话都被**禁止改 README**（热点文件由父会话在主树统一收尾），且都"只报可合并 tip"。

## 已核实的事实（父会话实测，可直接引用）

### 体积 / 依赖树基线
- C/C++ 编译单元 **6 个**（冷构建 `-p libscoop`：wall **157.9 s**，单元时长合计 455.4 s）：
  `unrar_sys` **75.8 s (C++)**、`libgit2-sys` 63.1、`zstd-sys` 45.4、`libssh2-sys` 34.2、`libsqlite3-sys` 12.6、`libz-sys` 9.8。
- release 静态库：unrar **3.66 MB** / git2 3.51 MB / sqlite3 2.26 MB / zstd 0.67 MB / ssh2 0.54 MB / z 0.14 MB；`target/release/hok.exe` **7.75 MB**（`lto="fat"` + `opt-level="z"` + `strip` + `panic=abort`）；`Cargo.lock` **331** 包。
- **git2 的边际依赖只有 4 个**（`git2`/`libgit2-sys`/`libssh2-sys`/`libz-sys`）；"54 crate 子树"里其余 ~50 个（`url`→`idna`→`icu_*`、`log`、`libc`、`bitflags`）被 reqwest / tower-http / rusqlite / crossterm / unrar 等共用 ⇒ (c)"子树大"是口径错觉，**瘦身空间≈0**。唯一可裁项是关 `git2` 的 `ssh` feature（−`libssh2-sys`，代价：SSH bucket 失效）。
- **rars 增量极小**：lock 331→333，仅 +`rars 0.9.4` +`zeroize_derive 1.5.0`（`aes 0.9.2`/`hmac 0.13`/`sha1`/`sha2 0.11`/`zeroize 1.9`/`aho-corasick`/`getrandom 0.4`/`rayon` **均已在树内**）⇒ 移除 `unrar`/`unrar_sys` 后依赖树**基本零增长**，同时去掉最大 C++ 编译单元。

### RAR
- **WinRAR 7.23 移除了 RAR4 创建**：`Rar.exe a -ma4` → `ERROR: Unknown option: ma4`（exit 7，父会话独立复现）⇒ **RAR4 覆盖只能靠 vendored 语料**（`vendor-rars/rar15_40/**`），它因此是承重的。
- fixture 生成脚本 `tests/fixtures/rar/gen-fixtures.ps1`：LCG 确定性内容、记录 WinRAR 版本与全部调用、声明"归档字节不可复现故只断言内容"。

### winsqlite3
- `C:\WINDOWS\System32\winsqlite3.dll` 实测 `sqlite3_libversion()` = **3.51.1**；SDK 10.0.26100.0 提供 `um\winsqlite\winsqlite3.h` 与 `um\{x64,x86,arm64}\winsqlite3.lib`。
- `libsqlite3-sys` 0.28.0 **没有 winsqlite3 feature**（0.25.2/0.26.0 有，0.28 起删除；master 0.38.1 也没有）。
- **build.rs 真实分支**（父会话逐行核验 `libsqlite3-sys-0.28.0/build.rs:49-85`）：
  `LIBSQLITE3_SYS_USE_PKG_CONFIG != 0 → build_linked`；否则 `bundled → build_bundled`（**完全不读 `SQLITE3_LIB_DIR`**，会静默编 C）；否则 `build_linked`。
- `build_linked::find_sqlite()`（`build.rs:436-471`）：设了 `SQLITE3_LIB_DIR` → 试 `<dir>/pkgconfig`，失败才 emit `rustc-link-lib=sqlite3` + `rustc-link-search=<dir>`；**什么都没设时仍 emit link-lib 并"hopes the library exists on the system paths"，不 panic** ⇒ **不是干净 fail-fast**，失败推迟到 link 期（`LNK1104`），最坏会静默链到别的 `sqlite3.dll`。
- 子代理实测（方案 A：保留 `bundled` + 三个 env 变量）：`cargo build -p libscoop` / `-p hok` exit 0；`libsqlite3-sys` 的 OUT_DIR 只有 `bindgen.rs`、无 `sqlite3.lib`；`dumpbin /dependents` 显示依赖 `winsqlite3.dll`。

### scoop.db 基线
- 只读副本 **`D:\hok-oracle-tmp\scoop.db`**（源 `D:\App\Scoop\scoop.db`，46.86 MB）：
  `app`=**21712**（extras 2388 / main 1654 / scoop-app 17612 / scoop-private 58）、`hok_manifest_sync`=**4**、`user_version`=0、`journal_mode`=**wal**、`quick_check`=ok。
- ⚠️ 该库含 `hok_manifest_sync` ⇒ 是 **hok 自己写过的**，"官方 Scoop(System.Data.SQLite) 写 → hok 读"这一支**未验证**，属残余缺口。
- 坑：android 的 `sqlite3.exe`（`D:\App\Scoop\apps\android-platform-tools\current\platform-tools\sqlite3.exe`）**打不开含非 ASCII 的路径**——用 ASCII 副本。

## 追加（2026-09-22，sqlite 子代理已交报告并转为 idle）

- 分支 `feat/winsqlite3` tip = **`c28d0bd`**（= 主线 `65b83ee` + 1 个测试提交 `test(cache): pin WAL so a reader snapshot survives a refresh`，+50/−0，已签名，工作树干净）⇒ **可直接 `--ff-only`，无需 rebase**。
- **三条新增硬事实（父会话已独立复核其中两条）**：
  1. `cargo build -p libscoop` 成功 ≠ 链接成功（libscoop 是 rlib 不链接）；缺 staging 的真实表现是 `cargo build -p hok` 最终链接 `rust-lld: error: could not open 'sqlite3.lib'`，`cargo check` 永远通过。
  2. **i686 链不了 winsqlite3**：MS 头文件在 x86 上把 `SQLITE_APICALL` 定为 `__stdcall` ⇒ x86 库 284/292 符号带 `@N` 修饰。**父会话独立复核**：x86 `winsqlite3.lib` 含 `_sqlite3_column_name@8` / `_sqlite3_libversion@0`（各 5 命中），x64 为 0 命中。CI 矩阵含 i686 ⇒ 任何"默认 winsqlite3"都要给 i686 显式 `--features bundled`（Cargo 无法按 target 自动切 feature）。
  3. 方案 D 的链接机制经**临时改动实测**（已还原）：无 `bundled` 时 libsqlite3-sys 只 emit `rustc-link-lib=dylib=sqlite3`（不 panic），search path 由 libscoop 自己的 `build.rs` 提供；rusqlite 0.31 对 3.14 预置绑定可干净编译。
- **验收全部通过**：双读一致（21642 行、SHA256 `9D8528DD…C1E975`，debug/release 同）、双向往返（populate + `hok update` 增量两条写路径，两方向逐字节一致）、`cargo test -p libscoop` **763 passed / 0 failed / 8 ignored**（新增 WAL 并发读测试）。
- **体积/时长实测（父会话独立复核过 exe 体积）**：release `hok.exe` **8,124,928 → 7,088,640 B（−12.8%）**；release 仅 sqlite 相关重建 269.63 s → 230.10 s。
- **已知既有问题（非本任务）**：`hok search '*'` 从 DB 读 21642 行 vs DB 表内 21712 行，缺口全在 scoop-app（70 行），两后端一致 ⇒ 与 SQLite 后端无关，**未定性**，建议另开线。
- **未验证**：ARM64 链接（被 `unrar_sys` 交叉编译阻断）、真机 i686、老 SDK、以及 B/C/D 的完整落地。
- 环境副作用：`rustup target add aarch64-pc-windows-msvc i686-pc-windows-msvc`（可 `rustup target remove` 回滚）。
- **父会话决定：暂不单独 ff 这个测试提交**——rars worktree 尚有未提交改动，此时推进主line 会让它 rebase 时面临脏工作树风险；等两条都就绪一并收口。

## 待决 / 待办

1. **winsqlite3 形态待用户拍板**（报告已到，父会话推荐 A + 两处 fail-fast 断言；D 仅在"要让 winsqlite3 当默认"时选）：
   - A 仅环境变量 + 构建脚本（依赖树零改动；缺环境会静默编 C，需 `LIBSQLITE3_SYS_USE_PKG_CONFIG=1` 才走 linked）；
   - B 显式 feature + 最小 patch/vendor `libsqlite3-sys`（可复现，长期维护一个 patch）；
   - C `.cargo/config.toml [env]` + staging 脚本 + CI 同步（免 per-shell env，缺 staging 硬失败需额外护栏）；
   - D 默认改 linked、`--features bundled` 退回（需解释 link 指令来源与 `build_linked` 行为）。
   另需结论：ARM64、老 Windows（winsqlite3.dll 版本更旧）策略；默认 `bundled` 构建**不得失效**。
2. **rars 侧硬缺口**：`crates/libscoop/tests/fixtures/rar/` **仍无 provenance README**；13 个 `vendor-rars/**` 第三方二进制**许可不明就不许提交**；`vendor-rars/rar13/WITHDIR.RAR` 与 `rar300/*` 必须先验证**旧 `unrar` 可读**才算 parity 用例（读不了只能标"旧实现不支持/rars 独有能力"）；filter 覆盖实际靠 vendor 的 `filter_*`/`rarvm_*`（自带 `rar5-filter.rar` 仅 0.2 KB）。
3. **合并后主树要做的 README 收尾**：`README-zh.md` 把 `zstd` 误标为"纯 Rust"（它是 C 绑定）、"RAR 无纯 Rust 替代"已过期（`rars` 存在）、`reqwest (rustls)`/`lzma-rs`/`dirs` 三处陈旧；另需记录 winsqlite3 的构建环境要求与"缺环境不 fail-fast"的坑。

## 收口顺序（父会话）

1. **落地检查单**：`git rev-parse <branch>` 指向子代理报的 tip；worktree **非 detached**（`rev-parse --abbrev-ref HEAD` = 分支名）；`git merge-base --is-ancestor refactor/edition-2024 <branch>` = true；主树未跟踪文档不变量 `git ls-files .dsh` = 0。
2. **线性 ff**：`pwsh "$HOME/.dsh/skills/worktree-parallel/scripts/worktree-merge.ps1" -Worktree .worktrees/<x> [-VerifiedRef <ref>]`
   后落地者先 rebase（本机 `rebase --continue` 不可用 → `cherry-pick -n` + `commit -C`），**`Cargo.lock` 重新生成而非手解**。
3. 主树补 README 收尾 → 在新树上重跑 `cargo test` + `cargo clippy -- -D warnings` 门禁。

## 本次已发生的"父会话判断被证据纠正"（如实记录）

1. "`SQLITE3_LIB_DIR` 单独设即可链接系统库" —— 错：`bundled` 在场时该变量**完全不被读取**，必须 `LIBSQLITE3_SYS_USE_PKG_CONFIG=1`。
2. "winsqlite3 不可行" —— 过大：feature 确已删除，但 DLL + SDK import lib 齐备，重命名 staging + 环境变量即可链接。
3. "`rar5-encrypted.rar` 没加密" —— 误报（我只看了 7z 的归档级字段）：文件级确为 `Encrypted = +`、`AES:15:3`。
4. rars 依赖树"净 −1 crate" —— 漏算 `zeroize_derive`，实际**净 ≈ 0**。

---

## 落地记录：winsqlite3（方案 A）已合并（2026-09-22）

- **主线 `refactor/edition-2024` = `eeef0a2`**（从 `65b83ee` 线性 ff，3 个提交、**无 merge commit**）：
  - `c28d0bd` test(cache): pin WAL so a reader snapshot survives a refresh
  - `bf07b40` build: add an opt-in winsqlite3 shape for the SQLite manifest cache
  - `eeef0a2` ci: guard the winsqlite3 shape on x64 pull requests
- 落地文件（6 个）：`.github/workflows/cicd.yml`、`.github/workflows/release.yml`、`.gitignore`、`crates/libscoop/build.rs`、`crates/libscoop/src/package/manifest_cache.rs`、`scripts/build-hok.ps1`。**不含 `Cargo.toml`/`Cargo.lock`**（方案 A 零依赖树改动）。
- 收口后**在新树上重跑门禁**：`cargo test -p libscoop --offline` → **763 passed / 0 failed / 8 ignored**（+ 集成 1/9/16 与 doctest 7 passed），`TEST_EXIT=0`；`cargo clippy -p libscoop --all-targets -- -D warnings` → `CLIPPY_EXIT=0`。落地检查单全过：worktree 非 detached、`merge-base --is-ancestor` 成立、`git ls-files .dsh` = 0、主树无未提交跟踪改动。
- **父会话独立复核**（非转述）：亲自跑 `pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify` → exit 0、`7,088,640 B`、`winsqlite3.dll imported` / `sqlite3.dll not imported`；逐行读过脚本与 `build.rs` 两处断言。
- 采用形态：**A**（默认仍 bundled；env 变量 + `scripts/build-hok.ps1` 走 winsqlite3；`build.rs` 两处 fail-fast；`release.yml` 用 `-Mode winsqlite3 -Verify`；`cicd.yml` 新增隔离 x64 job `ci_winsqlite3_shape`，**不动 i686/默认矩阵**）。
- 未验证（沿用子代理标注）：CI 上实跑耗时/green、ARM64 链接（交叉编译卡在 `unrar_sys` 的 `cl.exe`）、真机 i686、"官方 Scoop 写 → hok 读"、老 SDK。
- 环境副作用仍在：`rustup target add aarch64-pc-windows-msvc i686-pc-windows-msvc`（可 `rustup target remove`）；仓库外临时目录 `D:\hok-sqlite-wt\`、`D:\hok-winsqlite3-spike\`、`%TEMP%\hok-winsqlite3-spike\` 可随时删。

### 尚未做（等 rars 落地后一并收尾）
- 主树 README 收尾（**必须等 rars 落地后做**，因为依赖表里的 `unrar` 行会变）：`zstd` 误标纯 Rust、`RAR 无纯 Rust 替代` 过期、`reqwest (rustls)`/`lzma-rs`/`dirs` 陈旧、`scripts/build-hok.ps1` 用法与两种形状差异。
- rars 分支 `feat/rars-swap` 仍未提交（worktree 有未提交改动）；**主线已前进到 `eeef0a2`，它提交后需按 `cherry-pick -n` 重放**（已两次提醒：提交前禁止 rebase/`reset --hard`）。
