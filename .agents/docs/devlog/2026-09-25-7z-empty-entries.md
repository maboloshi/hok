# 7z 解压静默丢弃「无流条目」

> 2026-09-25 ｜ 空文件 + 目录）（1 提交 `4bf1397`，worktree `.worktrees/7z-empty-entries`，分支 `fix/7z-empty-entries`

**起因**：用户提问「解压模块适度切换到外部 7zip 的收益」→ 调研中做 hok 原生 vs `7z.exe` 26.03 的逐条对拍（报告 `.agents/docs/active/audit/7zip-switch-investigation-2026-09.md`），在真实缓存包 `nodejs#26.9.0#cde9f9d.7z` 上发现 **hok 比 7z.exe 少文件**，且差的都是 0 字节文件。

**根因**：`extract_7z_entries_mt` 为了块级并行用 `BlockDecoder::for_each_entries` 逐块解码（`internal/archive.rs:383`）——空文件与**所有目录条目**都没有数据流、不属于任何 block，于是**永远不会被访问**。sevenz-rust2 自己的 `ArchiveReader::for_each_entries` 在块循环之后有一段 "decode empty files" 兜底（`reader.rs:1655-1665`），hok 手写的块并行路径把它绕过了 ⇒ 静默丢条目、不报错。实测丢的是 `npm\.npmrc`、`node-gyp\gyp\pylib\packaging\py.typed`、`gyp\pylib\gyp\generator\__init__.py`、`smart-buffer\docs\ROADMAP.md`，以及**全部空目录**（目录条目同样无流，此前只靠写文件时隐式建父目录）。

**改动**：块 worker join 之后加一趟 stream-less 条目物化：`!entry.has_stream()` 的条目走与块路径同源的 filter / `strip_dir` 前缀剥离 / `ParentDir` 穿越检查；目录 → `ensure_dir`（7z.exe 会显式建空目录），空文件 → 写空文件并计入进度总数。纯函数级改动，不动引擎分派。

**验证**：① 单测 `test_extract_7z_empty_entries{,with_extract_dir}` + 206 B 只存（`-mx0`）夹具 `tests/fixtures/7z/empty-entries.7z`（2 个空文件 + 3 个目录，其中 2 个空）；把新 pass 临时 `take(0)` 关掉 → 两条用例**如期失败**（`empty file dropped: empty.txt` / `filtered empty file dropped`），恢复即绿。② worktree `cargo test -p libscoop` **771 passed / 0 failed / 9 ignored**（新增 2 条），`rustfmt --edition 2024 --check`、`cargo clippy -p libscoop --all-targets` 干净。③ 真实包端到端对拍 7z.exe 26.03（**含目录**）：nodejs **1934 → 2398 条目**（7z.exe 同为 2398），ffmpeg-essentials 49 条目；两者 `only-in-7z=0 / only-in-hok=0 / 类型与大小差异=0`。④ landed `refactor/edition-2024`（`4bf1397`；脚本漂移校验：1 提交 / 主题 / 3 文件集守恒 + 树逐字节相同，无 merge commit），**落地树 `cargo test --workspace` 全绿**（libscoop 771 passed / 9 ignored）。

**坑/教训**：① 「自己手写并行」会**悄悄丢掉库作者写好的兜底分支**——用 crate 的块级 API 时必须回头看它的 `for_each_entries` 尾部还做了什么（本次正是那段 "decode empty files"）；② 逐条对拍必须**把目录也算进去**：只比文件时，460 个目录条目的缺口完全不可见；③ 空文件在真实包里不是玩具（`py.typed`/`__init__.py`/`.npmrc` 是 node/Python 打包常客），"丢一个空文件"只在运行期炸，属于必须修的静默数据丢失；④ **全新 worktree 首次构建会 panic**（`crates/libscoop/build.rs:204` "hok-shim binary not found"）——先 `cargo build -p hok-shim`（该 worktree 的 target 里还没有 shim），细节已补进 `.agents/skills/hok-worktree-parallel.md`。
