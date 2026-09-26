# ZIP SFX 调研：rc-zip 对比与移植分析（2026-08-16）

> 背景：doubao 更新无进度条 → 根因 7z.exe 进度解析（`\r` 分隔）+ PE 前缀 ZIP
> 回退外部工具 → 实现原生 PE+ZIP (SFX) 提取 → 调研 rc-zip 现成方案 → 算法对比与修正。

## 1. 生态盘点：Rust 的 zip SFX 支持

| 库 | 版本/活跃度 | SFX/前缀数据 | deflate 后端 | 备注 |
|---|---|---|---|---|
| **zip (zip-rs)** 8.6 | 活跃 | ❌ `ZipArchive::new` 从 reader 起点解析，遇前缀失败 | flate2（hok 配 **zlib-rs**） | 需自研 EOCD 定位；上游自己处理前缀出过 bug（[zip-old#234](https://github.com/zip-rs/zip-old/issues/234)） |
| **rc-zip** 5.4 | 活跃（2025-11） | ✅ **内置** `global_offset` 自动推导 | 固定 **miniz_oxide** | sans-io + rc-zip-sync 封装；lzma 方法锁死 **lzma-rs**；zip64 完整 |
| zip-password-finder / safe_unzip 等 | — | 不适用 | — | 主题不匹配 |

**结论**：没有"给我一个 PE 文件自动解压"的一键 API。rc-zip 是唯一原生处理
前缀数据的，但它的 deflate 后端锁死 miniz_oxide、lzma 方法锁死 lzma-rs ——
恰好是 hok 之前特意换掉的两个后端。zip + 自研 EOCD 定位保持解码后端最优。

## 2. 算法对比：hok `find_embedded_zip_start` vs rc-zip `global_offset`

两者数学等价，但推导细节不同：

### rc-zip（`rc-zip/src/parse/eocd.rs:202-262`）

```rust
// located_directory_offset = EOCD 记录在文件中的绝对位置（扫描得出）
computed_directory_offset = located_directory_offset - directory_size;   // CD 绝对起点
if computed != recorded_directory_offset {                               // 有前缀
    global_offset = computed - recorded_directory_offset;                // 前缀大小
    // 修正后 entry 解析：header_offset += global_offset
}
```

- **先解析后修正**：`Archive::read` 时推导 `global_offset`，每个 entry 的
  `header_offset` 在 `resolve()` 里加偏移（`archive.rs:161-163`）。
- **布局假设**：`CD 结束 == EOCD 记录起点`——**comment 不参与**（comment 在
  EOCD 记录内部末尾，位于 CD 之后，不移动 CD 起点）。

### hok（修正后 `archive.rs`）

```rust
// eocd = EOCD 记录绝对位置（从文件尾反向扫描 PK\x05\x06）
cd_start = cd_end - cd_size;        // cd_end = EOCD 位置（普通）或 zip64 记录位置
zip_start = cd_start - cd_offset;   // 前缀起点 = 切片起点
// 然后 ZipArchive::new(Cursor::new(&data[zip_start..]))
```

- **先切片后解析**：算出 payload 起点，把前缀切掉交给 zip crate，zip crate 的
  `cd_offset` 自然相对正确。
- **zip64**：EOCD 字段为哨兵（0xFFFF / u32::MAX）时，经 `PK\x06\x07` locator
  定位 zip64 EOCD 记录（`PK\x06\x06`）取 64 位 `cd_size`/`cd_offset`，并以
  zip64 记录位置作为 CD 结束。

### 差异总结

| 维度 | rc-zip | hok |
|---|---|---|
| 处理时机 | 解析后修 `header_offset` | 解析前切 `data[start..]` |
| comment 处理 | 不参与（正确） | 修正前多减了 `comment_len`（**bug**），已修 |
| zip64 | 内置 | 本次新增（哨兵检测 + locator） |
| 等价性 | `global_offset = eocd_pos - cd_size - cd_offset` | 相同公式，仅应用方式不同 |

**移植可行性**：完全可行，且我们已经这么做了——修正后的 `find_embedded_zip_start`
与 rc-zip 的推导逐项一致（差异只在"切片 vs 加偏移"的落地方式，数学等价）。
无需引入 rc-zip 依赖（会带回 miniz_oxide / lzma-rs）。

## 3. 落地记录（commit）

- `f979730`：7z.exe 进度修复（`read_until('\r')` 解析 CR 分隔进度）；原生
  PE 前缀 ZIP 提取（`find_embedded_zip_start` + `extract_zip_embedded`），
  失败回退 7z.exe。
- `0b29111`：zip 解压并行化（per-entry worker 池，`ZipSource` 抽象文件/内存
  两种来源），doubao 100s → 25s（debug，16 核），与 7z.exe 逐文件一致。
- `b4f6e59`：修正 `find_embedded_zip_start` 推导（去掉多余的 `comment_len`，
  对齐 rc-zip 公式）+ zip64 支持。

## 4. 实测数据（doubao 2.24.7 缓存 `doubao#2.24.7#5cf1e4d.7z`）

- 614 MB PE+ZIP SFX（`MZ` 头，`Type=PE → zip`，14674 文件，解压 1.3 GB）
- 7z.exe 列表：14674 文件 / 1,306,617,306 字节
- hok 原生提取：一致；debug 单线程 100.4s → 并行 25.4s（~4x）

## 5. 后续可选

- 给 zip-rs 提 PR：移植 rc-zip 的 `global_offset` 算法（已证可行、规范），
  让 SFX 支持进入上游；hok 可随之简化 `find_embedded_zip_start`。
- 若未来有压缩比例敏感场景且可接受 miniz_oxide，可评估 rc-zip（当前不推荐）。
