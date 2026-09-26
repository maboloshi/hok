---
name: hok-nsis-extraction
title: NSIS 纯 Rust 提取（指令流目录跟踪已落地）
description: NSIS 纯 Rust 提取已落地验证（AltSnap 32/32 树一致）；solid 64MiB 预算静默截断为 P1 遗留。
metadata:
  type: project
---

NSIS 安装包纯 Rust 提取已落地（2026-08-12，c5b25ec + 8156e71，工作区干净）。背景：e00bc71 把 NSIS/Inno 排除出 7z SFX magic 搜索后 NSIS 走外部 7z.exe；用户要纯 Rust 方案，选现成库 `nsis`（ATRAPSLLC/nsis-rs，Apache-2.0，与 innospect 同作者）。

**实现（archive.rs）**：
- `extract_nsis`（镜像 extract_innosetup）：`nsis::NsisInstaller::from_bytes` → `installer.files()`（扫 EW_EXTRACTFILE）→ `name.to_string()`（Display 保留 $VAR 字面，勿用 to_path() 它会映射 $PLUGINSDIR→_plugins/）→ 路径净化 → decompress → 写盘
- `is_installer_pe` 拆出 `is_nsis_installer`；extract_7z dispatch：NSIS 标记 → extract_nsis 成功即返回，失败 fallthrough 7z.exe
- corpus 测试 `extract_nsis_electron_builder_corpus`（#[ignore]，样本=本机 `$env:SCOOP\cache\Obsidian#1.13.6#3cb95fb.7z` 即改名 NSIS 安装器，断言 $PLUGINSDIR\app-64.7z 存在）

**已验证**：Obsidian（electron-builder NSIS V3，331MB）：$PLUGINSDIR\app-64.7z（114MiB）字面目录写出、1.14 秒、与 7z.exe SHA256 完全一致。AltSnap（传统 NSIS V2）：内容与 7z 字节一致。⚠️ 本机 `$env:SCOOP\cache\Obsidian#1.13.6#3cb95fb.7z` 缓存样本已被清理（2026-08 复核不存在），跑 #[ignore] corpus 测试需先重新下载对应版本样本。

**✅ 挂起项已解决（2026-08-15）：传统 NSIS 目录结构差异**
- 问题：AltSnap 语言文件 7z 输出 `Lang\de_DE.ini`（目录），旧 nsis 库输出 `de_DE.ini`（扁平根目录）——语言文件失效
- **根因修正（旧结论错误！）**：旧调查说"目录来自 NSIS 文件表、SetOutPath 跟踪是错误方向"——**错**。实测（probe + track example，nsis crate 0.3.1 自带 fixture full_featured.exe）：header 布局 = 指令表后**紧跟字符串表**（probe 实测 entries end=7480 = strings offset=7480），**header 里没有文件表区域**；全表搜不到 `docs\payload.txt`，只有裸 `payload.txt`（字符串表两处）。**7-Zip 目录 = EW_CREATEDIR 指令流跟踪**：update_instdir=1 更新当前目录，EXTRACTFILE 拼当前目录，输出剥 `$INSTDIR` 前缀。旧探针只查 ASSIGNVAR/$OUTDIR，**漏查了 EW_CREATEDIR**（`SetOutPath <const>` 编译为单个 CREATEDIR，makensis 已优化掉 ASSIGNVAR，full_featured.nsi 证实）
- **修复实现（archive.rs）**：`extract_nsis` 三 pass——pass1 `nsis_extract_dirs(&installer)`：遍历 `installer.entries()`，`EW_CREATEDIR`(offset(1)!=0 更新) / `EW_ASSIGNVAR`(var==22 $OUTDIR) 跟踪当前目录，每个 `EW_EXTRACTFILE` 记录当时目录（流顺序）；pass2 仍用 `installer.files()`（复用 decompress），按序配对目录 → `nsis_dest_path(dir, name)`：name 含 `\`/`/` 用原名（$PLUGINSDIR\app-64.7z 字面保留），否则 `dir\name`，再剥 `$INSTDIR` 前缀；pass3 `installer.uninstallers()` 提取 Uninstall.exe（EW_WRITEUNINSTALLER，crate 跳图标前缀 + 拼安装器 PE stub；decompress 失败跳过不 Err）
- **AltSnap 1.67-x64 真实样本验证（D:\Downloads\AltSnap1.67-x64-inst.exe，NSIS-2 solid LZMA，189KB）**：与 7z x **32/32 文件树完全一致、31 文件 SHA256 字节一致**（含 Lang\ 18 个语言文件）；唯一字节差异 = Uninstall.exe（同为 33792B，前 30628B 与尾 32B 一致，中段 ~3KB 不同——crate 与 7z 的 overlay 生成细节差异，自包含可执行无碍）
- **⚠️ 连带修复：is_nsis_installer 识别不全**——AltSnap 的 stub **没有** "Nullsoft Install System" 字符串（只有 overlay FirstHeader magic `NullsoftInst` @ 0x7FF8+8），旧检测漏判 → 走 7z.exe 而非纯 Rust。已修：`is_nsis_installer` 增加 `NSIS_FIRST_HEADER_MAGIC = b"NullsoftInst"` 检测；`is_installer_pe` 统一复用 `is_nsis_installer`（防 AltSnap 误入内嵌 7z 搜索）
- **已知边界（旧版，详见下方「当前局限」）**：ASSIGNVAR($OUTDIR) 场景无实测样本（代码已处理）；相对 SetOutPath 按字面处理；Uninstall.exe 字节差异（crate vs 7z）；corpus 测试：`extract_nsis_traditional_dir_corpus`（NSIS_SAMPLE 环境变量，已验证 AltSnap 通过）
- 保持「extract_nsis 优先 + 7z 兜底」（纯 Rust，无 7z.exe 依赖）

## 当前局限（2026-08-15 复核，按影响排序，下次处理按此优先级）

**P1（已修复 2026-08-15，commit a1e99f0）：solid 全流 64 MiB 预算**——定性修正：crate 在 files() 迭代**明确报错**（非静默），修复前 hok 走 7z 兜底；已改用 `builder().max_decompressed_size(512MiB)`，makensis 3.12 编译的 72MiB solid 样本实测 SHA256 一致；上游 PR 2 仍值得提交（根治 `unwrap_or_else(Vec::new)` 吞错 + 文档化 Truncate）

**P2（行为边界，无实测样本，代码已部分处理）：**
- `ASSIGNVAR($OUTDIR)` 运行时路径：pass1 已处理（var==22 更新 cur_dir），但**无实测样本**——`SetOutPath $someVar` 动态路径安装器未验证
- 相对 `SetOutPath`（如 `"Lang"` 无 $INSTDIR 前缀）：按字面覆盖 cur_dir，若本意相对当前目录则丢上一级前缀；makensis 通常产出绝对/变量路径，罕见
- 运行时字符串运算路径（`$EXEDIR\..` 等）：静态跟踪只记字面，无法求值
- `${LANG:N}` 语言字符串路径：未验证

**P3（格式/资源/等价差异，有 7z 兜底或低风险）：**
- ~~zstd~~ **已澄清（2026-08-15）**：NSIS 官方无 zstd（makensis 3.12 实测 SetCompressor 仅 zlib|bzip2|lzma）——不存在此场景，上游 PR 3 已撤销
- 非流式：整文件载入内存 + 解压副本，几百 MB 安装器峰值高
- Uninstall.exe 与 7z 中段 ~3KB 生成差异（同为 33792B、前后一致；自包含可执行无碍）
- 空目录不落地（7z 显式建目录条目；hok 由写文件隐式建）
- `is_nsis_installer` 启发式：1MiB 扫描上限 + magic 匹配，超大 stub/魔改安装器可能漏判 → 7z 兜底（正确但非纯 Rust）

**Why:** 用户要求 NSIS 去 7z.exe 依赖；nsis crate 现成最优解，目录跟踪已补齐（7z 语义对齐）。
**How to apply:** NSIS 提取逻辑在 `internal/archive.rs` 的 extract_nsis/nsis_extract_dirs/nsis_dest_path；扩展注意：新指令流状态（如 ASSIGNVAR 其他变量）在 pass1 加分支、`files()` 与 entries 顺序配对（dirs 索引）；验证用 `test_nsis_dest_path` 纯函数 + NSIS_SAMPLE 环境变量的 corpus 测试 + 7z l 对比；勿用 `NsisString::to_path()`（$PLUGINSDIR→_plugins/ 映射，破坏字面语义）。**详细调研/局限/方案对照见 `.agents/docs/research/nsis-research.md`（§5 局限分级、§6 纯 Rust 方案盘点、§7 7z 局限、§8 下次处理）**；**上游提案草稿在 `.agents/docs/legacy/nsis-issue.md`（已归档）（2 个 PR：①ExtractedFile 目录跟踪 dest_path ②solid 静默失败修复；PR 3 zstd 已实证撤销）**。

## ⚠️ extract_7z 分流现状（2026-08-16 重构后，改动前必读）

> **函数已改名**：`extract_7z` → `extract_pseudo_7z`（2026-08-16，处理"扩展名 .7z 但内容未知"的伪 7z 分流，函数名不再声称只处理 7z）。

**当前最终分流**（commit `2679be7`）：① 7z magic → 纯 7z；② PE 非 installer + 内嵌 7z → 7z SFX；③ `classify_installer`（单遍扫 1MiB → `InstallerKind::{None,Nsis,Inno}`）→ NSIS/Inno 走 nsis/innospect crate；④ zip EOCD → zip crate 并行；⑤ 兜底 7z.exe。数据用 `Arc<Vec<u8>>` 全程共享（zip 分支 clone 不 move）。检测入口是 `classify_installer`——**不要再加 `is_nsis_installer`/`is_innosetup_installer` 独立谓词**（曾合并又删，死代码）。**Inno 主路径是 manifest `innosetup:true`（exe 文件），signature 分支只是 .7z 伪装的兜底。**

**反模式清单（反复重构教训，详见 `.agents/docs/reports/archive-refactor-retrospective.md`）**：
- 先鉴定文件真实格式（magic/EOCD/签名），别假设扩展名=格式（doubao 是 PE+zip 不是 7z）
- 数据要跨线程共享 → 一开始用 `Arc<Vec<u8>>`，别等 move 冲突再补
- 多格式检测 → 单遍 classifier 返回 enum，别按需拼谓词
- 分支前提互斥性先推演，防死代码（曾出现恒 false 的 NSIS 重试分支）
- 修 bug 只做最小修复，性能/重构另立提交；同一文件反复改注意范围蔓延
