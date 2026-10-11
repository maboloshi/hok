# `hok download` 主线程栈溢出（0xC00000FD）调研报告

- 日期：2026-09-25
- 分支/基线：`refactor/edition-2024` @ `d4a2b80`（worktree `.worktrees/dl-stack-overflow`，分支 `fix/download-stack-overflow`）
- 状态：**调研完成，等用户确认后再改代码**
- 相关技能：`investigate-first`、`hok-rust-gate-practice`

---

## 1. 现象与复现

```
$env:SCOOP=<tmp>\scoop; $env:SCOOP_GLOBAL=<tmp>\scoop_global; $env:HOME=<tmp>\home; $env:USERPROFILE=<tmp>\home
# min.json = {"version":"1.0","url":"http://127.0.0.1:1/x.bin","hash":"<64 个 0>"}

hok info     <min.json>  -> exit 0（正常打印）
hok download <min.json>  -> exit -1073741571 (0xC00000FD)
                            stderr: thread 'main' has overflowed its stack
hok download -s <min.json> -> 同样崩（与 -s/校验无关）
hok install  <min.json>  -> 同样崩
hok download <不存在路径>  -> exit 0（解析错误，不崩）
```

崩溃矩阵（实测，隔离环境 + 自行构建的 debug 二进制）：

| manifest | `info` | `download` |
|---|---|---|
| `{version,url,hash}`（min.json） | 0 | **0xC00000FD** |
| `{version,hash}`（无 url） | 0 | **0xC00000FD** |
| `{version,url}`（无 hash） | 0 | 0 |
| `{version}` | 0 | 0 |
| `{version,url,hash:"abc"}`（非法 hash） | 1（解析错误） | **0xC00000FD** |
| `{version,url,hash:"md5:…"}` | 0 | **0xC00000FD** |
| 不存在的路径 | — | 0 |

判别式：**manifest 里带 `hash` 字段** ⇒ `download`/`install` 崩，`info` 不崩。

## 2. 定位过程（证据）

用 eprintln 标记二分（诊断代码已全部 `git checkout` 还原）：

```
download_apps enter -> resolve_download_query enter -> parse_app(app=<path>, ver=None)
-> is_local=true -> get_manifest enter -> [installed 分支探测 2 次失败] -> not_installed 分支
-> parse_file enter <min.json>   ← 此后无输出即崩
```
⇒ 崩溃点在 **`parse_file` 内部的 `serde_json::from_str::<ManifestSpec>`**，不在包构造/事件循环/进度条。

放大 PE 头 `SizeOfStackReserve`（注意 PE32+ 该字段在 Optional Header `+0x48`；`+0x18` 是 `ImageBase`，别改错）：

| 命令（debug 构建） | 最小可用主线程栈 |
|---|---|
| `hok --version` / `hok home`（仅框架） | 768 KB |
| `hok info min.json` | 960 KB |
| `hok info real.json`（真实形状 manifest） | 1024 KB |
| `hok download nohash.json`（无 hash 字段） | 960 KB |
| `hok download min.json` | **1152 KB** ⇒ 默认 1024 KB 崩 |
| `hok download min.json`（`REGEX_HASH` 预编译） | 1024 KB |
| `hok download real.json` | **1216 KB** ⇒ 崩 |
| `hok install real.json` | **1216 KB** ⇒ 崩 |
| release 构建（以上全部） | 不崩（exit 0） |

默认二进制 PE 头 `SizeOfStackReserve = 0x100000 = 1 MiB`（实测）。

分解（64 KB 粒度）：

- **768 KB**：框架启动——语言探测 + `Cli::command()` clap 树构建 + `try_get_matches_from` + dispatch + logger（`src/cmd/mod.rs:558-601`）。**在跑任何业务代码前就已吃掉 75% 的主线程栈。**
- **+192 KB**：本地 manifest 解析（`get_manifest` → `not_installed` → `parse_file` → serde）。serde 侧实测：`parse_file` 入口到第一个字段反序列化帧就消耗 **123 KB**（`ManifestSpec` 派生 visitor + serde_json MapAccess，debug 帧肥大）。
- **+64 KB**：download 命令比 info 多的 async 层（`download_apps` → `resolve_download_query` → `resolve_isolated_query`，`crates/libscoop/src/package/download_apps.rs:135`、`manifest_source.rs:375`）。
- **+128 KB**：`HashString::new` 里 `REGEX_HASH.is_match(raw)`（`crates/libscoop/src/package/manifest.rs:1443`）触发 `LazyLock<Regex>`（`constant.rs:98`）**首次惰性编译**，恰好发生在最深调用点。

两个对照实验（决定性）：

| 实验 | 默认 1 MiB 下 `download min.json` |
|---|---|
| 基线 | 崩 |
| 在 `download_apps` 入口 `LazyLock::force(&REGEX_HASH)`（浅栈预编译） | **不崩** |
| 跳过 `REGEX_HASH.is_match`（临时 `false &&`） | **不崩** |
| 真实 manifest `real.json` + 预编译正则 | **仍崩**（1088 KB > 1024 KB） |

## 3. 根因

不是深递归/无界递归（2 MiB 即可跑通，且结果正确），而是 **debug 构建下主线程栈预算不足**：

- Windows 主线程栈来自 PE 头，默认 1 MiB，且**无法在运行时改变**；
- 本 CLI 在 debug（未优化）下的正常峰值：框架 768 KB + manifest 解析 192-448 KB + download 额外 async 层 64 KB + `REGEX_HASH` 首次编译 128 KB；
- 合法带 hash 的 manifest 使 `download`/`install` 峰值达到 **1.15-1.22 MiB** ⇒ 溢出；
- `info` 链更浅（960-1024 KB）刚好在悬崖内，所以同 manifest 不崩；
- release 构建（LTO/优化）栈用量远低，不崩。

即：**这是"预算"问题，不是某个函数写错**；正则惰性编译那 128 KB 只是压垮最小 manifest 的最后一根稻草（对真实 manifest 并非必要条件）。

## 4. 候选方案

| 方案 | 内容 | 代价/影响 | 能否单独满足验收 |
|---|---|---|---|
| **A. 不改** | 记为已知问题（debug 构建限制） | 验收明确要求跑通 | ✗ |
| **B. 提高主线程栈预留**（推荐） | `build.rs` 加 `println!("cargo:rustc-link-arg=/STACK:8388608")`（`CARGO_CFG_TARGET_OS == "windows"`） | +4 行；reserve 是虚拟内存，按需提交、零运行时开销；debug/release 都生效 | ✓（已实测 8 MiB 端到端成功） |
| **C. 大栈线程跑 runtime** | `bin/hok.rs` 去掉 `#[tokio::main]`，自建 runtime 与 8 MiB 线程，并可 `thread_stack_size` 抬高 worker | 改动 CLI 入口结构；跨平台；多一条线程；可一并覆盖 worker/blocking 线程 | ✓ |
| **D. 启动时预编译重正则** | `create_app` 开头 `LazyLock::force` 掉 `REGEX_HASH` 等 | 很小；消除 128 KB 尖峰 | ✗（真实 manifest 仍需 1088 KB） |
| **E. 削调用链/手写 hash 校验替掉正则** | 减少 async 层、去正则 | 改动面大；框架 768 KB 基线动不了 | ✗（info 本身已 960-1024 KB，无余量） |

## 5. 推荐

**B 为主**（栈预留 8 MiB，与 Linux 主线程默认一致）；如担心 tokio worker（2 MiB）与 libtest 测试线程（2 MiB）的同型风险，可改选/叠加 **C**。**D 不作为独立方案**（单独不够）；E 不推荐（收益不确定、改动面大）。原则：这里不存在"语义错位"，就是预算不足，抬高预算即正解。

影响面：
- `build.rs`（+4 行，cfg 守卫）；可选 `src/bin/hok.rs`（方案 C）。
- 回归测试：新增 CLI 级测试（必须跑真实 `hok.exe`，因为只有真实二进制的 PE 栈能复现；libscoop 单测线程为 2 MiB，永远复现不了本崩溃）。
  - 建议：`tests/download_stack_overflow.rs`，自带隔离 `Env`（参照 `tests/exit_codes.rs:20-81`）+ 一个一次性 loopback HTTP 服务器，断言 `hok download <带 hash 的本地 manifest>` **exit 0 且缓存文件落盘**（hash 用 `libscoop::hash::ChecksumBuilder` 现算）。
  - 备选：不带服务器，manifest 指向 `http://127.0.0.1:1/x.bin`，只断言 exit 0（更快更稳，同样能钉住本崩溃）。
- 一个主题一个提交：`fix(cli): reserve enough main-thread stack for the download path`。

## 6. 前后对照（可复跑）

```powershell
# 复现（基线 d4a2b80，debug）
cargo build -p hok-shim; cargo build --bin hok
$iso = "$env:TEMP\dlstack-iso"   # 造空目录 scoop/scoop_global/home + min.json
$env:SCOOP="$iso\scoop"; $env:SCOOP_GLOBAL="$iso\scoop_global"; $env:HOME="$iso\home"; $env:USERPROFILE="$iso\home"
.\target\debug\hok.exe download "$iso\min.json"; $LASTEXITCODE   # => -1073741571
```

```powershell
# 不重新编译即可验证"栈够用就正常"：把 SizeOfStackReserve(Optional Header +0x48) 改大
$b=[IO.File]::ReadAllBytes("target\debug\hok.exe"); $pe=[BitConverter]::ToInt32($b,0x3C)
[BitConverter]::GetBytes([uint64]8388608).CopyTo($b,$pe+24+0x48); [IO.File]::WriteAllBytes("$env:TEMP\hok-8m.exe",$b)
& "$env:TEMP\hok-8m.exe" download "$iso\min.json"; $LASTEXITCODE   # => 0（打印"下载失败"但正常退出）
```

端到端三态（本地限速 HTTP 服务器，已实测通过）：成功（exit 0 + 缓存落盘 + 进度行）、哈希失败（ignore_failures 下 exit 0 并报 `Hash mismatch`、错误文件被删）、缓存命中（第二次运行 0 次 GET，走 tail 校验）。服务器脚本见会话产物 `%TEMP%\dlstack-e2e\server.js`。

## 7. 待用户拍板的点

1. 选 **B**（build.rs `/STACK:`）还是 **C**（大栈线程）？栈大小取 **8 MiB** 可以吗？
2. 是否要一并做 **D**（启动预编译 `REGEX_HASH`）？我倾向不做（B 生效后无必要，单独又不够）。
3. 回归测试：带本地服务器的端到端版，还是 `127.0.0.1:1` 的极简版？
4. 是否本次一并处理 tokio worker / libtest 测试线程（2 MiB）的同型风险？（倾向仅登记，不改）

## 8. 落地结果（2026-09-25）

用户拍板：方案 **B** + 端到端回归测试；**不做 D**；worker/libtest 线程同型风险仅登记。

- 实现：`build.rs` 增加 `cargo:rustc-link-arg=/STACK:8388608`，守卫 `CARGO_CFG_TARGET_ENV == "msvc"`
  （`/STACK:` 是 link.exe / lld-link 参数；GNU-ld 风格链接器会把它当输入文件，故只在 msvc 目标下发出）。
  另：`build.rs` 原有的 `use` 顺序存在 rustfmt 既有偏差，本次按门禁把该改动文件 fmt 干净（同文件内 2 行重排）。
- 回归测试：新增 `tests/download_stack_overflow.rs`（CLI 级、跑真实二进制 + 一次性 loopback HTTP 服务器）：
  下载一个带 `hash` 的本地 manifest → 断言无栈溢出、exit 0、缓存落盘且内容一致；再跑一次走缓存命中。
- 验证：
  - 测试确实钉住崩溃——把 `target/debug/hok.exe` 的 `SizeOfStackReserve` 改回 1 MiB 后直接跑该测试
    二进制（绕开 cargo 重链接）→ **FAIL** 且报 `thread 'main' … has overflowed its stack`；改回 8 MiB → **PASS**（连跑 8 次稳定）。
  - 端到端三态（本地限速 HTTP 服务器，node）：成功 exit 0 / 2 请求 / 缓存落盘；哈希失败 exit 0 并报 `Hash mismatch` 且删除错文件；
    缓存命中 exit 0 且只有 1 次 HEAD（`正在从缓存加载 …`）。基线 1 MiB 副本在第一个请求前即崩。
  - `install` 不回归：同一 `.bin` manifest exit 0；另用合法 zip + `bin` 的 manifest 实测解压成功并生成 `stackapp.exe`/`shim`。
  - release 构建同样带 8 MiB 预留，`--version`/`info` 正常。
  - 门禁：`rustfmt --edition 2024 --check`（改动文件）0 差异；`cargo clippy --workspace --all-targets` 0 警告；
    `cargo test --workspace` exit 0（含新测试）。
- 落地：`fix/cli: reserve a real main-thread stack on Windows` = `6620d69`，
  经 `worktree-merge.ps1 -VerifiedRef 6620d69` 线性 ff 至 `refactor/edition-2024`（无 merge commit）；
  落地后在新树重跑 `cargo test --test download_stack_overflow` 通过、PE 预留 = 8 MiB、未跟踪文档仍为未跟踪。

**遗留（登记，不在本次范围）**：tokio worker 线程与 libtest 测试线程默认 2 MiB，理论上存在同型风险；
若将来出现 worker 侧深调用链，用 `Builder::thread_stack_size`（或 `RUST_MIN_STACK`）另开一题处理。
