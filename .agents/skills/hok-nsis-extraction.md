---
name: hok-nsis-extraction
title: NSIS 纯 Rust 提取
description: 改 internal/archive.rs 的 NSIS/伪 7z 解压分流，或排查安装器格式误判时读：extract_nsis 与上游 dest_path() 的目录语义、classify_installer 单遍分流、已知局限与反模式清单。
metadata:
  type: project
---

NSIS 走纯 Rust 提取（`nsis` crate），不再依赖外部 `7z.exe`。选它是因为它与 `innospect` 同作者、Apache-2.0，
且目录语义已由上游承担。

## 实现（`crates/libscoop/src/internal/archive.rs`）

- `extract_nsis`（与 `extract_innosetup` 同构）：`NsisInstaller::builder(&data).max_decompressed_size(512MiB).parse()`
  → `installer.files()` → `file.dest_path()` + `NsisString::to_install_path()` → 路径净化 → 并行 decompress → 写盘。
- **目录语义不要自己实现**：7z 形态的目录来自 **EW_CREATEDIR 指令流跟踪**（update_instdir=1 更新当前目录，
  EXTRACTFILE 拼当前目录，输出剥 `$INSTDIR`）；header 里**没有**文件表区域，别去那里找目录信息。
  上游 `dest_path()` + `to_install_path()` 已对齐 7z；本地那套三 pass 实现已删除，别再写回来。
- **勿用 `NsisString::to_path()`**：它会把 `$PLUGINSDIR` 映射成 `_plugins/`，破坏字面语义。
- `installer.uninstallers()` 用于提取 Uninstall.exe（decompress 失败跳过，不 Err）。
- NSIS 检测成功后即返回；失败 fallthrough 到 `7z.exe` 兜底。
- corpus 测试均 `#[ignore]`：traditional/solid 用例走 `NSIS_SAMPLE` 环境变量指定样本路径。

## 格式识别（单一入口）

- **入口是 `classify_installer`**：单遍扫 1 MiB → `InstallerKind::{None, Nsis, Inno}`；
  **不要再加 `is_nsis_installer` / `is_innosetup_installer` 独立谓词**（曾合并又删，是死代码）。
- stub 可能**没有** "Nullsoft Install System" 字符串，只有 overlay 的 `NullsoftInst` magic
  （`NSIS_FIRST_HEADER_MAGIC`）——只匹配前者的检测会漏判。
- Inno 主路径是 manifest 的 `innosetup: true`（exe 文件）；signature 分支只是 `.7z` 伪装的兜底。

## 最终分流顺序

① 7z magic → 纯 7z；② PE 非 installer + 内嵌 7z → 7z SFX；③ `classify_installer` → NSIS/Inno 走 nsis/innospect
crate；④ zip EOCD → zip crate 并行；⑤ 兜底 `7z.exe`。数据用 `Arc<Vec<u8>>` 全程共享（zip 分支 clone 不 move）。

## 已知局限（按影响排序）

**行为边界（无实测样本，代码已部分处理）**
- `ASSIGNVAR($OUTDIR)` 运行时路径：pass1 已处理（var==22 更新 cur_dir），但无实测样本。
- 相对 `SetOutPath`（如 `"Lang"` 无 `$INSTDIR` 前缀）：按字面覆盖 cur_dir，若本意是相对当前目录会丢上一级前缀；makensis 通常产出绝对/变量路径，罕见。
- 运行时字符串运算路径（`$EXEDIR\..` 等）：静态跟踪只记字面，无法求值。
- `${LANG:N}` 语言字符串路径：未验证。

**格式 / 资源 / 等价差异（有 7z 兜底或低风险）**
- 非流式：整文件载入内存 + 解压副本，几百 MB 安装器峰值高。
- Uninstall.exe 与 7z 输出的中段有 ~3 KB 生成差异（同尺寸、头尾一致；自包含可执行无碍）。
- 空目录不落地（7z 会显式建目录条目；hok 由写文件隐式建）。
- solid 解压预算：`max_decompressed_size(512MiB)` 由上游 `SolidStatus` + `Error::OutputTooLarge` 承担；
  迭代超限时上游**明确报错**（非静默），此时走 7z 兜底。
- NSIS 官方**无 zstd**（makensis 实测 `SetCompressor` 仅 zlib/bzip2/lzma）——不存在该场景。

## 反模式清单（反复重构的教训）

- 先鉴定文件真实格式（magic / EOCD / 签名），**别假设扩展名 = 格式**。
- 数据要跨线程共享 → 一开始就用 `Arc<Vec<u8>>`，别等 move 冲突再补。
- 多格式检测 → 单遍 classifier 返回 enum，别按需拼谓词。
- 分支前提互斥性先推演，防死代码（曾出现恒 false 的 NSIS 重试分支）。
- 修 bug 只做最小修复；性能/重构另立提交，同一文件反复改注意范围蔓延。

**How to apply:** 改 NSIS/伪 7z 分流前先确认上游 `nsis` 版本的行为；路径语义用上游 API，
验证用 `test_nsis_install_path_render` 纯函数 + `NSIS_SAMPLE` corpus + `7z l` 对比。
调研与方案对照见 [`.agents/docs/research/nsis-research.md`](../docs/research/nsis-research.md)；
上游提案记录在 [`.agents/docs/legacy/nsis-issue.md`](../docs/legacy/nsis-issue.md)（已归档，仅历史）。
