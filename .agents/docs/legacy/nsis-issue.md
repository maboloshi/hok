# 上游提案（nsis-rs + innospect）

> nsis-rs：https://github.com/ATRAPSLLC/nsis-rs（Apache-2.0，作者 BinFlip）
> nsis 版本基线：**0.4.0**（2026-08-15，已发布；0.3.1 已过时）
> innospect：https://github.com/ATRAPSLLC/innospect（原 BinFlip/inno-rs，已转移）
> innospect 版本基线：**0.1.4**（2026-09-18，已收录 PR #3）
> 状态（2026-08-16→17）：
> - **nsis-rs 0.4.0 已发布并吸收 hok 全部提案**：LZMA lzma-rs → lzma-rust2 + flate2 zlib-rs（commit 201df25，即原 PR #5）、`ExtractedFile::dest_path()`（原 PR 1 / issue #2）、solid 截断/解压失败不再静默（原 PR 2 / issue #3，`SolidStatus` + `Error::OutputTooLarge`），另含 NSIS 1.x 支持等大量修复；
> - **innospect [PR #3](https://github.com/ATRAPSLLC/innospect/pull/3)（已合并 → 0.1.4）**：同样的 lzma-rust2 + zlib-rs 切换（fork `maboloshi/innospect` 分支 `lzma-rust2`），2026-09-18 随 0.1.4 发布；上游另把 **LZMA2 字典大小改为读 Inno 的属性字节**（比 fork 的固定 32MiB 更强，`lzma2/ultra64` 类大字典包可正确解码）；
> - 早期 issue：nsis-rs [issues/2](https://github.com/ATRAPSLLC/nsis-rs/issues/2)（dest_path）、[issues/3](https://github.com/ATRAPSLLC/nsis-rs/issues/3)（solid 预算）、[issues/4](https://github.com/ATRAPSLLC/nsis-rs/issues/4)（lzma-rust2）；英文草案见 `nsis-issue-1-body.md` / `nsis-issue-2-body.md` / `nsis-issue-lzma-body.md`（同目录）
> 决策（2026-08-15→17）：dest_path / solid 预算 **暂不提 PR**，等维护者回应 issue #2/#3 的 API 方向；补丁 `nsis-pr1.patch` 已备好待用（同目录）。lzma-rust2 建议（issue #4）**已直接提 PR**（用户 2026-08-16 决定"直接 PR"）。
> **hok 侧（2026-08-17 / 2026-09-19 两次切回 registry）**：~~`crates/nsis` vendored fork~~ **已删除**——上游 0.4.0 吸收全部私有改动后改用 `nsis = "0.4"`（registry）；`archive.rs` 自研 `nsis_extract_dirs`/`nsis_dest_path` 目录跟踪已删除，改用上游 `ExtractedFile::dest_path()` + `NsisString::to_install_path()`（语义等价：剥 `$INSTDIR\` 前缀、`$VAR` 目录字面保留）。~~`crates/innospect` vendored fork~~ **已删除**（分支 `chore/innospect-registry`）——上游 0.1.4 收录 PR #3 后改用 `innospect = "0.1.4"`（registry）。
> 附注：曾计划 PR 3（zstd 解码），已实证撤销——见文末「已撤销提案」。

---

## PR 1：`ExtractedFile` 内置目录跟踪，提供带目录的最终路径

### 动机（源自原 issue：`ExtractedFile::name() drops subdirectory prefix (Lang\ vs flat)`）

传统 NSIS 安装器把文件装进子目录（如 AltSnap 的 `Lang\de_DE.ini`），目录信息不在
EXTRACTFILE 参数里，而是由 **`EW_CREATEDIR` 指令流**维护：

```
EW_CREATEDIR path="$INSTDIR"        update_instdir=1   ← 当前目录 = $INSTDIR
EW_EXTRACTFILE name="payload.txt"                       ← 7-Zip 输出 payload.txt
EW_CREATEDIR path="$INSTDIR\docs"   update_instdir=1   ← 当前目录 = $INSTDIR\docs
EW_EXTRACTFILE name="payload.txt"                       ← 7-Zip 输出 docs\payload.txt
```

`files()`/`ExtractedFile::name()` 只暴露 EXTRACTFILE 的裸目标名，目录结构丢失。
消费者（如 hok，一个 Rust Scoop 实现）被迫自研指令流跟踪（~80 行）才能复刻
7-Zip 的路径语义。

**实测根因（2026-08-15）**：目录**不是**来自 NSIS 文件表——header 中指令表后紧跟
字符串表，没有文件表区域；7-Zip 的 NsisIn.cpp 就是做指令流跟踪（EW_CREATEDIR +
EXTRACTFILE 拼接 + 剥 `$INSTDIR` 前缀）。旧版 issue 草稿的"文件表"推测有误，
以此为准。

### 建议方案

1. 在 `NsisInstaller`/`FileIter` 层维护 `current_dir`（遍历 `entries()`）：
   - `EW_CREATEDIR` 且 `update_instdir` 标志置位 → `current_dir = path`；
   - `EW_ASSIGNVAR` 目标为 `$OUTDIR`（变量索引 22）→ `current_dir = value`；
2. `ExtractedFile` 新增 `dest_path() -> Result<NsisString>`：
   - name 已含 `\`/`/`（如 `$PLUGINSDIR\app-64.7z`）→ 原样；
   - 否则 `current_dir\name`；
   - **剥 `$INSTDIR` 前缀**（7-Zip 语义：安装根 == 提取根）；
3. **保持 `name()` 现有语义不变**（向后兼容，不破坏现有消费者）。

### 验证

- fixture `full_featured.exe`：`dest_path()` 输出 `payload.txt` / `config.ini` /
  `docs\payload.txt`，与 `7z l` 逐项一致；
- 传统 NSIS 样本（AltSnap 1.67-x64，NSIS-2 solid）：与 7z x 文件树一致。

### 参考实现（hok 已在消费者侧按此逻辑实现并验证，见 commit 1db3bc4）

`src/installer/files.rs` 建议改动（`name()` 等现有 API 不变，向后兼容）：

```rust
// $OUTDIR is variable index 22 in the NSIS string table.
const OUTDIR_VAR: i32 = 22;

pub struct ExtractedFile<'a> {
    installer: &'a NsisInstaller<'a>,
    entry: Entry<'a>,
    /// The current install directory in effect at this EXTRACTFILE,
    /// tracked from EW_CREATEDIR / EW_ASSIGNVAR($OUTDIR).
    dir: NsisString,
}

impl<'a> ExtractedFile<'a> {
    /// 7-Zip-style extraction destination: `dir\name` with the `$INSTDIR`
    /// install-root prefix stripped (the install root is the extraction
    /// root). A name that already contains a directory separator
    /// (`$PLUGINSDIR\app-64.7z`) is returned verbatim.
    pub fn dest_path(&self) -> Result<String, Error> {
        let name = self.name()?.to_string();
        if name.is_empty() {
            return Ok(name);
        }
        let joined = if name.contains('\\') || name.contains('/') {
            name
        } else if self.dir.is_empty() {
            name
        } else {
            format!("{}\\{name}", self.dir)
        };
        Ok(joined
            .strip_prefix("$INSTDIR")
            .map(|rest| rest.trim_start_matches('\\').trim_start_matches('/'))
            .unwrap_or(&joined)
            .to_string())
    }
}

pub struct FileIter<'a> {
    installer: &'a NsisInstaller<'a>,
    entries: EntryIter<'a>,
    current_dir: Option<NsisString>,
}

impl<'a> Iterator for FileIter<'a> {
    type Item = Result<ExtractedFile<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let entry = match self.entries.next()? {
                Ok(entry) => entry,
                Err(e) => return Some(Err(e)),
            };
            match self.installer.normalize_opcode(entry.which()) {
                opcode::EW_CREATEDIR => {
                    if entry.offset(1) != 0 {
                        let path = match self.installer.read_string(entry.offset(0)) {
                            Ok(p) => p,
                            Err(e) => return Some(Err(e)),
                        };
                        self.current_dir = Some(path);
                    }
                }
                opcode::EW_ASSIGNVAR if entry.offset(0) == OUTDIR_VAR => {
                    let path = match self.installer.read_string(entry.offset(1)) {
                        Ok(p) => p,
                        Err(e) => return Some(Err(e)),
                    };
                    self.current_dir = Some(path);
                }
                opcode::EW_EXTRACTFILE => {
                    let file = ExtractedFile {
                        installer: self.installer,
                        entry,
                        dir: self.current_dir.clone().unwrap_or_default(),
                    };
                    if let Err(e) = file.validate_data_bounds() {
                        return Some(Err(e));
                    }
                    return Some(Ok(file));
                }
                _ => {}
            }
        }
    }
}
```

### 沟通要点

- 需要与上游确认：`dest_path()` 是否剥 `$INSTDIR`（hok 立场：剥，对齐 7z）；
- 是否同时提供 `NsisInstaller::file_entries_with_dirs()` 级别 API（便于不引
  `ExtractedFile` 的消费者）；
- 参考：hok 实现位于 `crates/libscoop/src/internal/archive.rs`
  （`nsis_extract_dirs` / `nsis_dest_path`，commit 1db3bc4）。

### 完整补丁（可应用，已验证）

`nsis-pr1.patch`（2 文件：`src/installer/files.rs` + `tests/integration.rs`，
基于 0.3.1；已在原始 0.3.1 副本上 `git apply --check` 通过，`cargo test` 全绿，
含新测试 `dest_path_tracks_createdir_directories`：full_featured →
`["config.ini", "docs\\payload.txt", "payload.txt"]` 与 7z 一致）。
上游回复需要时直接贴此 patch 或按其风格调整。

---

## PR 2：solid 全流解压失败/超预算不再静默

### 动机（P1：solid 安装器大文件在默认预算下失败）

`NsisInstaller::parse_with_budget`（`src/installer/nsisinstaller.rs:303-308`）对
solid 全流：

```rust
let full_stream = decompress::decompress_block(
    compressed_data,
    compression,
    DecodeLimit::Truncate(max_decompressed_size),
)
.unwrap_or_else(|_| Vec::new());   // ← 解压失败被吞
```

两个问题：
1. **超预算截断后错误含糊**：solid 全流被 `Truncate(64MiB)` 截断后，越界偏移的
   文件在 `FileIter::next` 的 `validate_data_bounds` 报 `TooShort`（实测：72MiB
   solid 安装器报 `file data payload: expected at least 75497476, got 67104998`）——
   错误类型是"数据越界"，没有指出根因是预算截断，消费者难定位；
2. **解压失败静默**：`unwrap_or_else(|_| Vec::new())` 把任何失败变成空
   `solid_data` → 所有文件 `data()` 为空，提取"成功"但全丢。

与 0.3.0 已对齐的单文件行为**不一致**：单文件用 `Capped` + `Error::OutputTooLarge`
（明确拒绝，见 CHANGELOG 0.3.0）。

**hok 侧缓解（2026-08-15，commit a1e99f0）**：`extract_nsis` 改用
`builder().max_decompressed_size(512 MiB)`——72MiB solid 样本实测提取完整
（SHA256 一致）。但上游根治（错误语义 + 吞错）仍值得提交。

### 建议方案

1. solid 全流解压**失败** → 传播错误（去掉 `unwrap_or_else(|_| Vec::new())`），
   `files()` 迭代时暴露；
2. 超预算**截断** → 越界文件的 `decompress()` 返回 `Error::OutputTooLarge { limit }`
   （对齐单文件），或 `FileIter` 检测 solid 流被截断并报错；
3. 至少：文档化 `Truncate` 的截断语义 + 消费者如何感知截断。

### 验证

- 用 `builder().max_decompressed_size(小)` 构造超预算 solid 样本：
  修复前静默空/TooShort，修复后明确 `OutputTooLarge`；
- 回归：现有 solid fixture（full_featured 等）不回归。

### 沟通要点

- `Truncate` 是 0.3.0 引入的设计（solid 缓冲按偏移切片，宁可截断不失败）——
  需与上游确认截断时的错误语义（`OutputTooLarge` vs 文档化+越界报错）；
- hok 侧现状：`extract_nsis` 用 `from_bytes()`（默认预算），已计划改用
  `builder().max_decompressed_size(512MiB)` 缓解——但上游修复才是根治。

---

## PR 3（已提交）：LZMA 解码 lzma-rs → lzma-rust2 + flate2 zlib-rs

> **状态（2026-08-16）：已直接提 PR，两个仓库均已提交**——
> [nsis-rs #5](https://github.com/ATRAPSLLC/nsis-rs/pull/5)（OPEN）、
> [innospect #3](https://github.com/ATRAPSLLC/innospect/pull/3)（OPEN）。
> 由 issue [nsis-rs #4](https://github.com/ATRAPSLLC/nsis-rs/issues/4) 演进而来，
> 用户 2026-08-16 决定跳过等回应、直接提 PR；两个 fork 分支均为 `lzma-rust2`。

### 动机（性能）

`lzma-rs` 是纯 Rust 但多年未积极优化。`lzma-rust2` 是活跃维护的解码器，
hok 实测 **LZMA1 约快 1.4 倍**（21MB 流 0.79s → 0.68s），纯 Rust 无 C 依赖；
`flate2` 默认 `rust_backend`（miniz_oxide）也换成更快的 **zlib-rs** 后端。

### 改动（两个仓库同构）

- `Cargo.toml`：`lzma-rs = "0.3.0"` → `lzma-rust2 = { version = "0.18", default-features = false, features = ["std", "optimization"] }`；`flate2` → `default-features = false, features = ["zlib-rs"]`；**lzma-rs 降为 dev-dependency**（测试夹具仍用它压缩）；
- `src/decompress/lzma.rs`（nsis）：`lzma_rs::lzma_decompress` → `lzma_rust2::LzmaReader::new_mem_limit` + `io::copy`；
- `src/decompress/block.rs` + `src/extract/chunk.rs`（innospect）：`UseProvided(None)` → 5 字节 Inno 头补 `u64::MAX` 哨兵成 13 字节 LZMA-Alone 头 + `LzmaReader`；LZMA2 分支 → `Lzma2Reader`（32MiB dict，Inno 1 字节 prop 前缀跳过）。

### 行为说明

- 解码器在 EOS 标记处停止，尾随字节（CRC、padding）不再触发 lzma-rs 的
  "more bytes are available" 错误——数据产出相同；
- 公开 API 不变，`Error::Decompress` 映射保留；
- 验证：真实安装器树一致 + 单元测试全绿（nsis 205 个、innospect 79 个）。

### hok 侧对应

- `crates/nsis`（commit 93fb7f6 + eb44383）、`crates/innospect`（commit d629b70）已 vendor 相同改动；
- 上游合并后可将 `path` 依赖切回 registry（`nsis = "0.3"` + `innospect = "0.1"`）。

---

## 已撤销提案：PR 4 zstd 解码支持（2026-08-15 实证推翻）

曾计划为 NSIS 3.1+ 的 zstd 压缩加解码支持（接入纯 Rust 的 ruzstd）。**实证结论**：

- 用最新官方 makensis（NSIS 3.12）实测：`SetCompressor` 只接受
  `zlib|bzip2|lzma`——**NSIS 官方不支持 zstd**；
- NSIS v312 源码（`Source/` 目录）无任何 zstd 集成；
- 结论：**不存在 zstd 压缩的 NSIS 安装器**，该 PR 动机不成立，撤销。
- 教训：此前"NSIS 3.1+ 支持 zstd"为误传（与 7-Zip 的 zstd 支持混淆）；
  若将来 NSIS 官方加入 zstd，可重新评估（ruzstd 0.9.0 方案仍有效）。

---

## 不构成 PR 的项（hok 已自行处理）

- `is_nsis_installer` FirstHeader-only stub 检测（hok 侧 dispatch 逻辑，crate 的
  `scan_for_first_header` 已覆盖识别）；
- Uninstall.exe 提取（crate 已有 `uninstallers()` + `decompress()` API，hok 仅消费）。
