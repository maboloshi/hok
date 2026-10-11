# NSIS 提取、兼容修复、进度条与 7z 性能

> 2026-08-15

## 1. NSIS 纯 Rust 提取（4 commit：`1db3bc4` `7aaef75` `298eae9` `a1e99f0`）

**根因修正（推翻旧结论）**：传统 NSIS 安装器的目录结构**不是**来自"NSIS 文件表"——header 里指令表后紧跟字符串表，无文件表区域（probe 实测 full_featured：entries end=7480 = strings offset=7480，全表无 `docs\payload.txt`）。7-Zip 的目录 = **EW_CREATEDIR 指令流跟踪**（update_instdir 更新当前目录 + EXTRACTFILE 拼接 + 剥 `$INSTDIR` 前缀）。旧探针漏查 EW_CREATEDIR（`SetOutPath <const>` 编译为单个 CREATEDIR，makensis 已优化掉 ASSIGNVAR）。

**实现（三 pass）**：pass1 `nsis_extract_dirs` 指令流跟踪；pass2 `files()` 配对目录（`nsis_dest_path`）；pass3 `uninstallers()` 提取 Uninstall.exe。

**连带修复**：`is_nsis_installer` 只认 stub 字符串 `Nullsoft Install System`，漏判 FirstHeader-only stub（AltSnap 无该字符串，只有 `NullsoftInst` magic @0x7FF8+8）→ 增加 `NSIS_FIRST_HEADER_MAGIC` 检测。

**验证**：AltSnap 1.67（NSIS-2）与 7z x **32/32 文件树一致、31 文件 SHA256 一致**（Uninstall.exe 中段 ~3KB 生成差异无碍）；makensis 3.12 现场编译样本通过；solid 预算 64MiB 截断定性修正（files() 明确报错 → 7z 兜底，非静默损坏）。

**已撤销的 PR 3**：zstd 解码（NSIS 3.1+）——**实证 NSIS 官方无 zstd**（makensis 3.12 `SetCompressor` 仅 zlib|bzip2|lzma，v312 源码无 zstd 集成），此前"NSIS 3.1+ zstd"为误传（与 7-Zip 的 zstd 混淆）。

## 2. 兼容修复（2 commit：`2207a8c` `57e23ae`）

- **`$fname` 变量缺失**：installer.script 环境没定义 `$fname`（Scoop 语义：第一个下载 URL 文件名）→ 大量 manifest 的 `Expand-InnoArchive "$dir\$fname"` 退化为目录路径 → innounp exit 3。修复：`run_script` 注入 `$fname = '<url文件名>'`（与 `copy_downloaded_files` 目标名一致）。实测 `hok update zed` 1.11.3→1.15.0 成功。
- **running 过滤前置**：`update *` 且 `ignore-failures=true` 时，确认列表仍显示被 skip 的包（`Transaction.upgrade` 是 OnceCell，组装后过滤无效）→ 过滤提前到 transaction 组装前。实测列表只剩实际可升级项。

## 3. 进度条（2 commit：`d1865f8`）

- **覆写残留**：`print!("\r ...")` 不清行 → `\r\x1b[2K` 先清整行；
- **性能**：逐文件事件（git 9567 次 print+flush）→ `emit_extract_progress` 节流（每 50 + 最后）；进度显示改为**计数**（`包名 n/total`，去文件名刷屏）；
- **实测**：10000 文件 zip，节流进度开销仅 **4.2%**（18.8→19.6s）；修复前估算 25-50%（stdout flush 主导）。

## 4. 7z 提取性能（2 commit：`8812596` `0c05848`）

**性能链路**：逐文件 `read_file`（solid 灾难 >10min）→ `for_each_entries`（每块一次，63s）→ **块级并行**（`BlockDecoder` worker 池，20.3s @release）。

**⚠️ 最大教训：性能基准必须 `--release`**。之前 37s 全是 `cargo test`（debug）假象——lzma-rust2 的 `optimization` feature（unsafe 汇编）仅 release 生效。release 下 git 包（56MB/6 solid 块/9567 文件）：串行解码 11.1s、并行 9.0s、**全量 20.3s，优于 7z.exe（29.3s）44%**。

**实验排除**：
- xz2（C liblzma）：同一 LZMA1 流 lzma-rust2 快 14%（0.68 vs 0.79s）——纯 Rust 已是最优，不换；
- BCJ2/Delta filter 链：纯 LZMA vs BCJ2 归档无差距；
- 依赖升级 sevenz-rust2 0.21.4/lzma-rust2 0.18.1：无性能变化。

## 5. 上游协作（ATRAPSLLC/nsis-rs）

- 提交 2 个 issue：[#2](https://github.com/ATRAPSLLC/nsis-rs/issues/2)（`dest_path()` 目录跟踪，含参考实现 `.agents/docs/legacy/nsis-pr1.patch`）、[#3](https://github.com/ATRAPSLLC/nsis-rs/issues/3)（solid 预算语义）；
- 决策：**暂不直接提 PR，等上游回应 API 方向**（补丁已备好）；
- 详细调研：`.agents/docs/research/nsis-research.md`；提案：`.agents/docs/legacy/nsis-issue.md`（已归档）。

## 教训汇总
1. 纯 Rust 解码器（lzma-rust2）可能快于 C（liblzma）——**先实测再假设**；
2. 性能数据必须标注编译模式（debug/release）；
3. 格式逆向先探针实证（probe/track），勿凭旧结论（文件表假设误判 3 天）；
4. OnceCell 型 transaction 组装后不可改——前置过滤；
5. PowerShell 传含 `$` 的 commit message 用单引号。

## 6. 其他解压模块审计（commit `c1643bb`）

按 7z 的四个检查点（重复解压/并行/编译模式/解码器）审计全部解压模块：

| 模块 | 解码器 | 结论 |
|---|---|---|
| zip | flate2 **zlib-rs** ✓ | 已最优；entry 独立可并行但 deflate 快、IO 主导，收益小不做 |
| tar.gz | flate2 zlib-rs ✓ | 已最优 |
| tar.bz2 | bzip2 crate（C libbz2）✓ | 已最优 |
| **tar.xz** | ~~lzma-rs 0.3~~ → **lzma-rust2**（`c1643bb`） | **实测 1.52x 提速**（1.28→0.84s，21MB）+ 流式（原整读内存）；SHA256 验证一致 |
| tar.zst | zstd crate（C）✓ | 已最优 |
| rar | unrar 0.5（纯 Rust） | 未测；RAR 包少见，低优先 |
| nsis | nsis crate 内部 lzma-rs 0.3 | 不能换（crate 内部）；NSIS LZMA 包少见，已知限制 |
| innosetup | innospect 内部 | 未深测；zed 安装已验证可用 |

- 通用结论：除 tar.xz 外各模块已用最优解码器（zlib-rs/C 绑定）；无 7z 式"重复解压"灾难（zip 按 entry seek、tar/rar 流式、nsis/innospect 迭代一次）；并行收益仅对解码慢的大 7z（已做）。

## 7. NSIS 解码器 vendor（commit `93fb7f6` + 上游 issue #4）

- **问题**：nsis crate 内部用 lzma-rs 0.3（2023 停更）——同 7z 场景的慢解码器嫌疑；
- **验证**：临时 fork 换 `lzma-rust2`（`LzmaReader::new_mem_limit` + `io::copy` 替换 `lzma_rs::lzma_decompress`）——**162 单元 + 31 集成 + 12 doctest 全绿**（EOS/预算语义保持，尾部字节容错分支对 unknown-size 成死代码）；21MB LZMA1 流实测 **~14% 快**（0.68 vs 0.79s）；
- **决策：vendor + 上游建议**——`crates/nsis`（nsis-rs 0.3.1 fork，改 2 处），libscoop 切 path 依赖；上游 [issues/4](https://github.com/ATRAPSLLC/nsis-rs/issues/4) 已提（采纳后可切回 registry）；
- **vendor 维护注意**：上游更新需手动同步（对比 `crates/nsis` vs ATRAPSLLC/nsis-rs main）；我们的 hok 侧改动仅 `decompress/lzma.rs` + `Cargo.toml` 依赖。

## 8. innospect vendor + miniz_oxide 出树（commit `d629b70`）

- **目标**：全项目 flate2 统一 zlib-rs，miniz_oxide 出树；
- **来源排查**：miniz_oxide 原由 innospect（flate2 默认）**和 ureq（默认 features 含 gzip → flate2 默认）** 引入——zip 已 `default-features=false` ✓；
- **改动**：vendor `crates/innospect`（inno-rs 0.1.2 fork，同 BinFlip）：flate2→zlib-rs、**LZMA1/LZMA2 都换 lzma-rust2**（LZMA1 用 5B 头+8B sentinel 补 13B alone 头；LZMA2 用 `Lzma2Reader` dict 默认 32MB，流内 chunk header 可重置；lzma-rs 保留为 dev-dependency 仅供测试生成 fixture）；ureq 改 `default-features = false`（关 gzip → 不发 `Accept-Encoding: gzip`，服务器回明文，安全；保留 rustls/json）；
- **验证**：`cargo tree` **miniz_oxide 已出树** ✓（flate2 全树仅 zlib-rs）；innospect 79+6+1 全绿（fixtures 覆盖 LZMA1/LZMA2 solid）；workspace 全绿；
- **⚠️ 已知独立问题**：zed（Inno LZMA2 大包）端到端提取 93s 后报 `NotADirectory`（某 entry 目标父目录是文件——路径冲突，非解码错误，解码全成功）；innospect fixtures 已覆盖 LZMA1/LZMA2 解码正确性，zed 路径冲突为既有 bug 待处理。

## 9. 上游 PR：nsis-rs #5 + innospect #3（2026-08-16）

- **背景**：lzma-rust2 建议（issue #4）用户决定"直接 PR 不等回应"；
- **nsis-rs [PR #5](https://github.com/ATRAPSLLC/nsis-rs/pull/5)**：`Cargo.toml`（flate2 zlib-rs + lzma-rust2）+ `src/decompress/lzma.rs`（`LzmaReader::new_mem_limit` + `io::copy`）；lzma-rs 降 dev-dep（测试压缩 fixture 用）；205 测试全绿；
- **innospect [PR #3](https://github.com/ATRAPSLLC/innospect/pull/3)**：`block.rs`/`chunk.rs`/`Cargo.toml` 同构切换（Inno 5B LZMA1 头补 `u64::MAX` 哨兵成 13B alone 头；LZMA2 用 `Lzma2Reader` 32MiB dict）；79 测试全绿；
- **仓库转移发现**：原 `BinFlip/inno-rs` 已转至 **ATRAPSLLC/innospect**（0.1.3 起），fork `maboloshi/innospect`；
- **CRLF 陷阱**：fork clone 检出 CRLF 是 autocrlf 转换残留，blob 实为 LF → 干净 clone + `--config core.autocrlf=false` 后 diff 干净；nsis 仓库 blob 是 CRLF 则反之（保持 CRLF 提交）；
- **hok 侧**：`crates/innospect` 对齐 0.1.3（`e3c1ec1`：ATRAPS LLC 版权/URL/pascalscript 0.1.2/NOTICE），`Cargo.lock` 重新生成（`161f311`，330→318 包，lzma-rs 仅 dev-dep 保留）。

## 10. doubao 进度条根因 + zip SFX 原生解压 + zip 并行（3 commit：`f979730` `0b29111` `b4f6e59`；三者均为拆分前的原始提交号，本节末尾有勘误）

**用户报告**：`hok update doubao`（2.23.7→2.24.7）解压无进度条，缓存 `doubao#2.24.7#5cf1e4d.7z`。

**根因链**：
1. doubao 下载 URL 是 `Doubao_installer_*.exe#/dl.7z`——**PE 前缀 ZIP（SFX）**：614MB，`MZ` 头，7z.exe 识别 `Type=PE → zip`（14674 文件，解压 1.3GB）；前 1MB 无 NSIS/Inno/7z magic → hok 回退 7z.exe；
2. 7z.exe 回退路径进度**永远发不出**：`-bsp1` 的进度输出用 `\r` 分隔、**从不换行**，旧代码 `BufRead::lines()` 按 `\n` 分割 → 整个流缓冲到进程退出 → 零进度事件（git 包走原生 7z 路径所以一直正常）。

**修复 1（`f979730`）**：`extract_with_7z_exe` 改 `read_until(b'\r')` 逐段解析；实测 7z stderr 原始字节确认全是 CR 无 LF。

**修复 2（`f979730`）原生 PE 前缀 ZIP 提取**：`find_embedded_zip_start` 从文件尾扫 EOCD（`PK\x05\x06`），`cd_start = eocd_pos - cd_size`，`zip_start = cd_start - cd_offset`（cd_offset 相对 payload 起点）；`extract_zip_embedded` 从切片起 `ZipArchive::new`。失败回退 7z.exe（不硬错误）。doubao 实测 zip_start=10613506，与 EOCD 位置吻合。

**修复 3（`0b29111`）zip 并行解压**：zip entry 自带 local header 无解码依赖 → per-entry worker 池（`available_parallelism` 个，原子索引动态领取）；`ZipSource` enum 统一 文件/内存 两来源（`ZipArchive` 非 Sync，每 worker 独立实例）；embedded 路径 move 缓冲避免复制 614MB。**doubao debug 100.4s → 25.4s（~4x，16 核）**，14674 文件/1.3GB 与 7z.exe 逐文件一致。

**修复 4（`b4f6e59`）算法修正**：原 `eocd - comment_len - cd_size - cd_offset` **多减了 comment_len**——comment 在 EOCD 记录内部（CD 之后），不移动 CD 起点；修正为 rc-zip 同款 `cd_start = eocd_pos - cd_size`（见 `.agents/docs/research/zip-sfx-research.md` §2 算法对比）；顺带 zip64 支持（EOCD 哨兵值 → `PK\x06\x07` locator → zip64 EOCD 记录取 64 位值）。

**生态结论（调研详见 `.agents/docs/research/zip-sfx-research.md`）**：rc-zip 5.4 原生支持前缀数据（`global_offset`），但 deflate 锁死 miniz_oxide、lzma 锁死 lzma-rs（hok 特意换掉的后端）→ 不引入；zip + 自研 EOCD 定位保持最优后端，算法与 rc-zip 数学等价。**不构成提交项**：`src/scoop_handler.rs` 遗留（去 Done 输出）按用户要求保留未提交。

> **提交号勘误**：上文本节写的 `f979730`/`0b29111`/`b4f6e59` 是拆分前的原始提交；最终提交序列为 `b268f63`（7z.exe 进度）→ `58e82ef`（SFX 提取+推导修正+zip64）→ `379b7b0`（zip 并行）→ `5fa74c3`（分流平行化+Arc 共享+worker 排序，doubao 21.1s）→ `5cb14a6`（单遍 classifier+Inno 兜底+伪 7z 改名，原 `2679be7` 经 2026-08-16 amend）。反复重构的教训复盘见 `.agents/docs/reports/archive-refactor-retrospective.md`。

## 11. 剩余格式多线程评估（2026-08-16，结论：维持原状）

**背景**：7z（block 级）与 zip（entry 级）并行落地后，评估其余格式是否可多线程化。

**评估表**（关键判据=该格式是否有**独立的并行单元**）：

| 格式 | 并行单元 | 可并行? | 结论 |
|---|---|---|---|
| 7z | 独立 block（各自 LZMA 流） | ✅ 已做 | — |
| zip | 每文件独立 local header | ✅ 已做 | — |
| tar.bz2 | bzip2 独立 block | ⚠️ 可（lbzip2 纯 Rust） | 价值低：Scoop 生态罕见，且当前 bzip2 是 C 绑定 |
| tar.gz / tar.xz | 单一连续流（deflate/LZMA1） | ❌ 格式限制 | 无块边界可并行 |
| tar.zst | 帧内块可并行，但无纯 Rust 成熟解码器 | ❌ | ruzstd 单线程 |
| tar（无压缩） | 纯 IO | — | 瓶颈在磁盘，并行无意义 |
| rar / nsis / innospect | 无（流式/内部 solid 单流） | ❌ | 与 7z 本质区别：无 block 边界 |

**核心结论**：7z/zip 能并行是因为**解压与解码都有独立单元**；其余格式要么单
连续流（压缩层无法并行）、要么纯 IO（不需要并行）、要么内部 solid 单流
（nsis/innospect，文件级并行无意义）。**已覆盖生态中"值得并行"的全部场景，
剩余格式多线程化投入大收益近零，维持原状。** 若将来遇到 tar.bz2 大包卡顿，
可针对性评估 lbzip2。

---
