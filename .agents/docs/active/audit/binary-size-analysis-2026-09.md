# hok 构建产物（二进制）体积分析 — 2026-09-24

> **一句话结论**：7.08 MB 是「静态链接 libgit2 + Rust HTTP/正则/解压栈」的**结构性体积**，没有单一开关能把它显著变小；
> 现阶段真正有杠杆的是**分发形态**（裸 exe 7.08 MB → 7z **2.26 MB** / zip **3.12 MB**），
> 以及把 2026-09 那次「`cargo bloat` 40 分钟跑不完」的归因工作换成**已可复现的 PDB 归因**（本文方法）。
> 本文只做调研，不含任何代码改动。

## 1. 实测现状（本机、HEAD `675b991`）

| 对象 | 形态 | 大小 | 证据 |
|---|---|---|---|
| `target/release/hok.exe` | winsqlite3 + fat LTO + opt-level=z + panic=abort + strip | **7,084,544 B（6.76 MiB）** | 本次重建；sha256 `CBC1FF53A2977604179223DF04890DBD28DF39D8E1035E39EA3401D02904D083` |
| 同 profile、bundled SQLite（参考值） | 内置 SQLite 编译 | 8,124,928 B | `scripts/build-hok.ps1:94-97`（脚本注明 informational、旧提交实测；差 ≈ **1.04 MB**） |
| `hok-shim.exe` / `hok-shim-gui.exe` | `no_std` shim | 各 11,776 B | 两者被 `include_bytes!` 嵌进 hok.exe（`crates/libscoop/build.rs:177-218`）⇒ 合计仅 **23 KB** |
| 分发压缩：`zip -mx9` | — | **3,122,078 B（44.1%）** | 本机 7z.exe/`ZipArchive` 实测 |
| 分发压缩：`7z -mx9` | — | **2,264,717 B（32.0%）** | 同上，压缩耗时 ≈3 s |

段分布（`llvm-objdump -h`，本次重建）：

| 段 | 字节 | 占比 |
|---|---|---|
| `.text` | 5,326,894 | 75.2% |
| `.rdata` | 1,522,268 | 21.5% |
| `.pdata` | 149,172 | 2.1% |
| `.reloc` | 42,092 | 0.6% |
| `.data` | 38,391 | 0.5% |
| 其余（`.tls`/`_RDATA`/`.fptable`） | 1,268 | <0.1% |

⇒ `.text + .rdata` = **96.7%**；`.rdata` 里 1.38 MB 是「非字符串字面量」的只读数据（vtable/跳转表/查表数据，如 Unicode、regex 自动机、i18n 表）。

**发布与分发路径**：`.github/workflows/release.yml:39` 走 `pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify`，`:45-47` 只上传裸 `target/release/hok.exe`（无压缩包）。
**仓库自身无大文件**：最大的被跟踪文件是 532 KB 的测试夹具 `crates/libscoop/tests/fixtures/scoop-original/decompress/TestCases.zip` ⇒ 体积问题只在 exe 本身。

构建耗时（顺带记录，推翻旧笔记里「40 分钟」的印象）：改 `libscoop` 后 fat-LTO 发布重建 **220.7 s**；`target\size-probe` 从零全量 thin-LTO 构建 **257 s**。

## 2. 归因方法（替代 `cargo bloat`）

三层，全部本机实测：

1. **段级**：`llvm-objdump -h`（见上表）。
2. **成分级（C 库可分离）**：PDB 的 section contributions —— `llvm-pdbutil dump -section-contribs`，本次覆盖 7,080,085 B ≈ exe 全量（误差 <0.1%）。
3. **crate 级（Rust）**：**fat LTO 下做不到** —— 34,437 个符号全部归到单个 `hok.hok.a001d0d09f742152-cgu.0.rcgu.o`（79.7%），因为 fat LTO 把整个依赖图的 bitcode 合成一个模块。
   因此改用 **thin-LTO 探针**：`CARGO_TARGET_DIR=target\size-probe` + `CARGO_PROFILE_RELEASE_LTO=thin`，其余与发布 profile 相同、同为 winsqlite3 形态，再按 PDB `module → Obj(rlib)` 映射归因。
   探针 7,589,376 B vs 发布形态 7,084,544 B ⇒ **fat LTO 相比 thin 省 506 KB（6.7%）**（即现状已是对的）。

### 2.1 发布形态（fat LTO）可见成分

| 成分 | 大小 | 占比 |
|---|---|---|
| 合并 LTO 对象（**全部 Rust crate 之和**，无法再分） | 5,514 KB | 77.9% |
| libgit2_sys (C) | 784 KB | 11.1% |
| MSVC CRT（libucrt/libvcruntime/libcmt） | 212 KB | 3.0% |
| zstd_sys (C) | 98 KB | 1.4% |
| libssh2_sys (C) | 96 KB | 1.4% |
| libz_sys (C) | 44 KB | 0.6% |
| 其余（linker 段、绝对符号等） | 166 KB | 2.3% |

> 注意：C 静态库的 `.a/.lib` 体积（libgit2 3.69 MB、zstd 0.70 MB、ssh2 0.57 MB、z 0.14 MB）**不等于**最终占用——链接器的函数级清理后只剩上面这些。

### 2.2 crate 级（thin-LTO 探针，占比为近似值）

家族聚合（贡献总额 7,580,842 B）：

| 家族 | 大小 | 占比 |
|---|---|---|
| **一方代码**（libscoop 1,194 KB + hok 439 KB + hok bin CGU 299 KB） | ≈1.93 MB | 25.5% |
| **regex 栈**（regex_syntax 484 + regex_automata 402 + aho_corasick 155 + memchr 19 KB） | 1.06 MB | 14.0% |
| **libgit2 + libssh2**（785 + 97 KB） | 0.88 MB | 11.6% |
| **解压/归档栈**（rars 273、innospect 169、zstd_sys 98、nsis 67、zlib_rs 56、libz_sys 44、lzma_rust2 37、tar 31、libbz2_rs_sys 29、zip 20、sevenz_rust2 18、crc32fast 18 KB…） | ≈0.86 MB | 11.3% |
| **解析层**（goblin 126、jsonpath_rust 129、sxd_xpath 96、sxd_document 78、icu_normalizer 77、json5 51、serde_json 36、url 30、idna 30、idna_adapter 34、pest 27 KB…） | ≈0.76 MB | 10.0% |
| **Rust 运行时 / CRT / linker**（std 225、core 100、CRT 212、linker 171、alloc 14、rustc_demangle 26 KB…） | ≈0.75 MB | 9.9% |
| **CLI / 输出 / i18n**（clap_builder 165、hok_i18n 120、clap_complete 60、tracing_subscriber 60、indicatif 45、console 16 KB…） | ≈0.5 MB | 6.6% |
| **HTTP/TLS/async**（reqwest 229、tokio 74、hyper 48、hyper_util 44、http 35、mio 21 KB…） | ≈0.45 MB | 6.4% |

单个 crate Top：`libscoop` 1,194 KB › `libgit2_sys` 785 KB › `regex_syntax` 484 KB › `hok` 738 KB（两段 CGU）› `regex_automata` 402 KB › `rars` 273 KB › `reqwest` 229 KB › `std` 225 KB › `innospect` 169 KB › `clap_builder` 165 KB › `aho_corasick` 155 KB › `jsonpath_rust` 129 KB › `goblin` 126 KB › `hok_i18n` 120 KB。**前 21 个 crate 覆盖 80.6%**。

## 3. 与参照的对比

- **官方 Scoop（PowerShell）**：整仓 2.3 MB / 142 文件，但**不自包含**——依赖系统 PowerShell 5.1 与全部 `lib/`+`libexec/` 脚本；hok 是自包含单文件（唯一外部依赖是 Windows 自带的 `winsqlite3.dll`）。两者不可直接比大小。
- **unrar 已彻底移除**：`Cargo.toml`/`Cargo.lock` 无任何 `unrar` 引用（`target` 里的 `libunrar.a` 是陈旧产物）——这是此前一次真实瘦身（RAR 改纯 Rust `rars`）。
- **SQLite 形态已优化过**：bundled → winsqlite3 省 ≈1 MB，且发布路径已固定用 winsqlite3。

## 4. 候选方案与代价

| # | 方案 | 收益（实测/估算） | 代价/风险 | 建议 |
|---|---|---|---|---|
| A | **不改** | 0 | 无 | 基线成立：7 MB 是静态链接 + 零运行时依赖的合理代价 |
| B | **分发改压缩包**（release asset 由裸 exe 换成 zip/7z） | 下载/资产 **7.08 MB → 2.26 MB（7z，−68%）/ 3.12 MB（zip，−56%）** | 用户多一步解压；需决定是否保留裸 exe asset 兼容既有脚本；`release.yml:45-47` 一行级改动 | **推荐先做** |
| C1 | 关掉 `git2` 的 `ssh` feature（去 libssh2） | −**96 KB**（C）+ 少量 crypto 代码 | 与 `.agents/docs/active/plan/private-bucket-plan-2026-09.md` 的私有 bucket SSH 计划直接冲突 | 不做（除非放弃 SSH bucket） |
| C2 | 去 `goblin` | −**126 KB** | 不是直接依赖：`cargo tree -i goblin` = `innospect 0.1.4` / `nsis 0.4` 的传递依赖 ⇒ 只能上游 PR，本仓无法直接删 | 记录，不做 |
| C3 | 裁 regex 的 Unicode/大小写折叠 | 数百 KB 量级 | 会改变大小写不敏感匹配语义（Scoop manifest 匹配依赖它）；risky | 不做 |
| C4 | `sxd-xpath`+`sxd-document`（174 KB）/ `jsonpath_rust`（129 KB）换自研裁剪 | ~300 KB | 仅 `checkver` 用；解析细节风险高、维护面变大 | 不做 |
| C5 | 去 `rars`(273 KB)/`innospect`(169 KB)/`nsis`(67 KB) | ~0.5 MB | 三者正是「纯 Rust 解压、无 C++ 工具链」的既定决策；功能平价必需 | 不做 |
| D | 调 profile（`opt-level="s"` / `lto="thin"` / 去 LTO） | **负收益**：实测 thin 比 fat 大 506 KB；`s` 通常大于 `z`；去 LTO 更大 | 唯一收益是构建时间（thin 探针全量 257 s） | 不做（现状已最优） |
| E | feature-gate 可选命令做 "lite" 构建（checkver/xpath 等） | **实测可干净 gate 的部分仅 ≈363 KB（thin 探针）= 4.8%**，换算发布形态 ≈340 KB ⇒ 7.08 → 6.75 MB（原估 0.5–1 MB 偏乐观，已修正）。再想多只能砍解压格式（rars/innospect/nsis/zstd/lzma/bz2/tar/7z ≈720 KB = 9.5%），那等于放弃 RAR/Inno/NSIS/zst 平价 | 断面不干净：`checkver::apply_autoupdate` 被 `manifest_source.rs:461 generate_user_manifest` 调用（`install app@version` 路径）；命令注册由 `build.rs`→`__cmd_reg__.rs` 自动生成，gate 需同时改 build.rs/clap 枚举/`hok-i18n-derive` 三处；测试与 CI 矩阵翻倍；两个 release asset | 不做（收益已被 B 吞掉：lite 压缩后 ≈2.1 MB vs 全量 7z 2.26 MB） |
| F | UPX 之类运行时压缩 | exe 面值下降 | 杀软误报、破坏签名、启动解压 | 不做 |

## 5. 推荐

1. **先做 B**（压缩分发）：这是唯一「用户可感知体积」的改动；7z 已在本机可用（`7z a -t7z -mx=9`），zip 无外部依赖（`Compress-Archive`/`ZipArchive`）。需要先定 zip/7z 与是否保留裸 exe。
2. **归因结论入档**（本文）并更新 `.agents/docs/active/plan/pending-tasks.md` 的 EN5 行：原记录「`cargo bloat` 实测 40 分钟未完成已中止」的方法已废弃，改为 PDB 归因；结论是**体积结构性、profile 不再调**。
3. **代码级瘦身暂不动**（C1–C5 收益/风险比差，C2 需上游）。
4. 若以后还要压体积，唯一有量级空间的方向是**砍功能面**（SSH bucket / RAR 平价 / checkver xpath），那属于产品决策，不属于「构建优化」。

## 6. 待确认（请拍板）

1. 分发形态：**zip**、**7z**、还是两者都给（并保留裸 exe）？
2. 归因脚本是否落库（目前仅在 `%TEMP%`：`hok-size-crates.ps1`/`hok-size-families.ps1`/`hok-size-map.ps1`，含 `llvm-pdbutil` 调用与分组规则）？
3. 本次探针产物 `target\size-probe\`（**1.2 GB**）保留（便于复测）还是删除？

## 附：复现命令

```powershell
# 1) 发布形态（winsqlite3，PDB 保留，供归因用）
pwsh scripts/build-hok.ps1 -Mode winsqlite3 -Verify

# 2) 段级
llvm-objdump -h target\release\hok.exe

# 3) 成分级（C 库 / 段归属），llvm-pdbutil 在 $env:SCOOP\apps\llvm\current\bin\
llvm-pdbutil dump -modules          target\release\hok.pdb > mods.txt
llvm-pdbutil dump -section-contribs target\release\hok.pdb > contribs.txt

# 4) crate 级：thin-LTO 探针（fat LTO 会把 Rust 全并成一个对象）
$env:CARGO_TARGET_DIR = "$PWD\target\size-probe"
$env:CARGO_PROFILE_RELEASE_LTO = 'thin'          # 其余 profile 键同 Cargo.toml
$env:LIBSQLITE3_SYS_USE_PKG_CONFIG = '1'         # 与发布形态对齐（winsqlite3）
$env:SQLITE3_LIB_DIR = "$PWD\.winsqlite3"; $env:SQLITE3_INCLUDE_DIR = "$PWD\.winsqlite3"
cargo build -p hok-shim --release
cargo build -p hok --release --bin hok
# 对 target\size-probe\release\hok.pdb 重复第 3 步，再按 module->Obj(rlib) 聚合
```
