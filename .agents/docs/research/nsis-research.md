# NSIS 提取调研与技术记录（hok 侧）

> 本文档存放 hok 对 NSIS 安装器提取的**全部调研与实现记录**（2026-08-12 起，2026-08-15 复核）。
> 与 `.agents/docs/legacy/nsis-issue.md`（给 nsis-rs 上游的 PR/issue 草稿，已归档）分离：本文档是 hok 内部工作记录。
> 工作记忆入口：`.agents/skills/hok-nsis-extraction.md`；本文档是详细版。

## 1. 结论速览

- hok 用纯 Rust（`nsis` crate 0.3.1 + 自研指令流跟踪）提取 NSIS，7z.exe 仅兜底；
- 核心修复：**7-Zip 的目录结构来自 `EW_CREATEDIR` 指令流跟踪，不是文件表**（旧结论错误，见 §2）；
- 已实证：传统 NSIS（AltSnap，Lang\ 18 语言文件）与 electron-builder（$PLUGINSDIR）与 7z 输出一致；
- 剩余唯一高影响项：**solid 全流 64 MiB 预算静默截断**（§5 P1）。

## 2. NSIS 格式与 7-Zip 行为（根因修正）

**旧结论（2026-08-12，错误）**：7-Zip 的目录来自"NSIS 文件表"，`SetOutPath` 跟踪是错误方向，正解=补文件表解析（对标 NsisIn.cpp，大工程）。

**实测结论（2026-08-15，probe + track example 实证）**：
- header 布局：指令表（entries block）后**紧跟字符串表**，**没有文件表区域**（probe：full_featured.exe entries end=7480 = strings offset=7480；全表搜不到 `docs\payload.txt`，只有两处裸 `payload.txt`）；
- **7-Zip 目录 = 指令流跟踪**：`EW_CREATEDIR`（update_instdir=1）更新当前目录 → `EW_EXTRACTFILE` 拼接 → 输出剥 `$INSTDIR` 前缀；
- 旧探针只查 `ASSIGNVAR/$OUTDIR`，**漏查 `EW_CREATEDIR`**——`SetOutPath <const>` 编译为单个 CREATEDIR（makensis 已优化掉 ASSIGNVAR，full_featured.nsi 证实）；
- 变体：`EW_ASSIGNVAR($OUTDIR)`（var==22）用于运行时路径；`create_directory`（update_instdir=0）不改变当前目录。

**其他格式事实**：
- FirstHeader 签名：`flags + 0xDEADBEEF + "NullsoftInst"`（512 对齐扫描 + 非对齐回退）；NSIS 1.x 为 `nsisinstall\0`、旧版 `NullSoftInst`（NSIS-Dumper 列出 4 种变体）；
- 压缩：LZMA（默认）/ deflate / bzip2 / none；**zstd（NSIS 3.1+）nsis crate 不支持**；
- solid / non-solid 两种模式；文件条目 4 字节长度前缀（bit31=压缩标志）。

## 3. hok 实现（archive.rs，commit 1db3bc4 / 7aaef75 / 298eae9）

`extract_nsis` 三 pass（纯 Rust，无 7z.exe）：

- **pass1 `nsis_extract_dirs(&installer)`**：遍历 `installer.entries()`，跟踪当前目录——`EW_CREATEDIR`（offset(1)!=0 更新）/ `EW_ASSIGNVAR`（var==22 $OUTDIR 更新）；每个 `EW_EXTRACTFILE` 记录当时目录（流顺序）；
- **pass2**：`installer.files()`（复用 crate 解压）按序配对目录 → `nsis_dest_path(dir, name)`：name 含 `\`/`/` 用原名（`$PLUGINSDIR\app-64.7z` 字面保留），否则 `dir\name`，再剥 `$INSTDIR` 前缀；
- **pass3**：`installer.uninstallers()` 提取 Uninstall.exe（EW_WRITEUNINSTALLER；crate 跳图标前缀 + 拼安装器 PE stub；失败跳过不阻塞）。

**连带修复 `is_nsis_installer`**：AltSnap 的 stub **没有** "Nullsoft Install System" 字符串（只有 FirstHeader magic `NullsoftInst`）——旧检测漏判走 7z.exe。已增 `NSIS_FIRST_HEADER_MAGIC = b"NullsoftInst"` 检测；`is_installer_pe` 统一复用 `is_nsis_installer`。

## 4. 验证结果

| 样本 | 格式 | 结果 |
|---|---|---|
| nsis crate fixtures ×4（full_featured/ansi_deflate/deflate_solid/lzma_nonsolid） | NSIS2 ANSI / NSIS3 Unicode / solid / non-solid | track example **逐项复现 `7z l`** |
| **makensis 3.12 现场编译样本（`SetCompressor lzma` + `docs\` 目录，最新官方工具链）** | NSIS-3 Unicode | corpus 测试通过（`docs\payload.txt` 正确提取） |
| **makensis 3.12 现场编译样本（`SetCompressor /SOLID lzma` + 72MiB payload，bigtest.exe）** | NSIS-3 Unicode solid | P1 实证：默认预算 files() 报错；builder 512MiB 修复后 `big.bin` 72MiB **SHA256 一致**（corpus 测试通过） |
| AltSnap 1.67-x64（`D:\Downloads\AltSnap1.67-x64-inst.exe`，NSIS-2 solid LZMA，189KB） | 传统 NSIS | 与 7z x **32/32 文件树一致、31 文件 SHA256 字节一致**（含 Lang\ 18 个语言文件）；Uninstall.exe 同为 33792B，前 30628B+尾 32B 一致，中段 ~3KB 生成差异（自包含可执行无碍） |
| Obsidian（electron-builder，2026-08-12 曾验证） | NSIS3 + $PLUGINSDIR | `$PLUGINSDIR\app-64.7z` 字面写出、与 7z SHA256 一致 |

测试：`test_nsis_dest_path`（纯函数 6 断言）、`test_is_installer_pe`/`test_pe_embedded_sfx_searchable`（FirstHeader 检测）、`extract_nsis_traditional_dir_corpus`（`NSIS_SAMPLE` 环境变量，已验证 AltSnap 通过）、`extract_nsis_electron_builder_corpus`（#[ignore]，样本需重新下载）。

## 5. 当前局限（下次处理按此优先级）

**P1（已修复，2026-08-15）：solid 全流 64 MiB 预算**（原"静默截断"定性已修正）
- **实测定性（makensis 3.12 编译 72MiB solid 样本 bigtest.exe）**：crate 在 `files()` 迭代时 `validate_data_bounds` **明确报错**（`file data payload: expected at least 75497476, got 67104998`），并非完全静默——修复前 hok `extract_nsis` 失败 → **7z.exe 兜底（正确但非纯 Rust）**，不会装坏文件；
- **已修复（commit a1e99f0）**：`extract_nsis` 改用 `NsisInstaller::builder(&data).max_decompressed_size(512 MiB)`——bigtest.exe 实测提取成功，`big.bin` 72MiB **SHA256 与原始一致**（44.8s）；
- 512 MiB 覆盖现实安装器（Obsidian electron-builder payload ~114MiB）；极端 >512MiB 仍失败 → 7z 兜底；
- 上游修复（PR 2：去 `unwrap_or_else(Vec::new)` 吞错 + 文档化 Truncate）仍值得提交——hok 侧缓解 ≠ 上游根治。

**P2（行为边界，无实测样本）**：
- `ASSIGNVAR($OUTDIR)` 运行时路径（代码已处理，无样本）；
- 相对 `SetOutPath`（如 `"Lang"`）按字面覆盖 cur_dir，可能丢上一级前缀（makensis 通常产出绝对路径，罕见）；
- 运行时字符串运算路径（`$EXEDIR\..` 等）只记字面；
- `${LANG:N}` 语言字符串路径未验证。

**P3（有兜底/低风险）**：
- ~~zstd 压缩~~ **已澄清（2026-08-15 实证）**：NSIS 官方无 zstd——makensis 3.12 实测 `SetCompressor` 仅接受 `zlib|bzip2|lzma`，v312 源码无 zstd 集成；不存在 zstd NSIS 安装器（此前"NSIS 3.1+ zstd"为误传，与 7-Zip 的 zstd 混淆）；上游提案 PR 3 已撤销；
- 非流式：整文件载入内存；
- Uninstall.exe 中段 ~3KB 生成差异；
- 空目录不落地（7z 显式建目录条目）；
- `is_nsis_installer` 1MiB 扫描上限 + magic 启发式（漏判 → 7z 兜底）。

## 6. 纯 Rust 方案调研（GitHub 盘点）

| 项目 | 能力 | 结论 |
|---|---|---|
| [nsis-rs](https://github.com/BinFlip/nsis-rs)（ATRAPSLLC，Apache-2.0） | 完整解析+提取（hok 在用） | 主方案，无同级替代 |
| [newtua-formats](https://github.com/trankov/newtua-formats)（LGPL-3.0，未上 crates.io） | XADMaster 移植：指令流目录跟踪 + **目录条目生成** + 变量展开；`$PLUGINSDIR` 展开为符号占位 "NSIS Plugins Directory" | 路径解析思路与我们一致（互证）；**$PLUGINSDIR 占位破坏 electron-builder + LGPL 不兼容 Apache-2.0 → 不可用**；目录条目生成可借鉴 |
| [NSIS-Dumper](https://github.com/0v41n/NSIS-Dumper)（MIT） | 149 行零依赖取证工具：签名定位 → 整体 dump 成 .7z；无路径解析 | 仅 4 种 FirstHeader 签名变体有参考价值 |
| cargo-packager / tauri-bundler | **生成** NSIS 安装器（非解析） | 排除 |
| hyperscoop（hp，本地对照） | **内置 7z.exe/7z.dll**，全部走外部 7z；无 Rust NSIS 解析 | 反方向路线（若放弃纯 Rust 可参照） |

## 7. 7z.exe 对 NSIS 的局限（兜底路径能力边界）

- **官方 7-Zip 自 15.05 起禁用 NSIS 脚本解包**（`NsisIn.h` `// #define NSIS_SCRIPT` 默认注释，只能提取文件；第三方构建 [myfreeer/7z-build-nsis](https://github.com/myfreeer/7z-build-nsis) 取消注释才有 `[NSIS].nsi` 伪文件）——对 hok 无影响（不需脚本反编译）；
- **必须 7z.dll**：7za/7zr 独立版**不识别 NSIS**；
- **静态提取固有边界**（与 hok 纯 Rust 一致，非 7z 独有缺陷）：不执行安装逻辑（注册表/快捷方式/插件）、条件分支不评估（未执行分支的文件也提取）、运行时变量路径输出占位/失真；
- 工程面：外部进程开销、错误靠 exit code、版本差异大（历史 regression）、zstd NSIS 支持未实证。

## 8. 下次处理建议

1. **P1 solid 预算**：`builder().max_decompressed_size(512MiB)` + 大 solid 样本实测（唯一高影响）；
2. P2：收集 `ASSIGNVAR($OUTDIR)` / 相对路径真实样本验证；
3. （可选增强）借鉴 newtua 的目录条目生成、NSIS-Dumper 的签名变体检测（NSIS 1.x）；
4. ~~zstd~~ 已撤销（NSIS 官方无 zstd，见 §5 P3）。

## 9. 关联

- 工作记忆：`.agents/skills/hok-nsis-extraction.md`
- **上游提案草稿：`.agents/docs/legacy/nsis-issue.md`（2 个 PR：①目录跟踪 API ②solid 静默失败修复；PR 3 zstd 已实证撤销）**——提交上游前按该文件「沟通要点」逐项确认
- 提交：`1db3bc4`（指令流跟踪）/ `7aaef75`（uninstaller 提取）/ `298eae9`（FirstHeader 检测）
