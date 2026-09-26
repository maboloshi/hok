# 解压模块「适度切换到外部 7z」收益调研（2026-09）

> 触发：用户提问「对比 scoop，分析一下我们解压模块适度切换到外部 7zip 的收益」，
> 随后追加「或者可以考虑 7z.dll 作为解压后端 而且是可以共用线程?」。
> 本文只做调研与选项对照，**未改任何仓库代码**；实施范围待用户确认（`investigate-first`）。

## 0. 一句话结论

- **性能不是收益**：同机同输入实测，hok 原生路径在 6 个真实缓存包里有 4 个明显快于 `7z.exe 26.03`
  （最快 7.4×），只在「少文件、大 solid/大块 LZMA」的样本上慢 ~1.9×。
- **真正的收益是「兜底与保真」**：① 原生路径失败/未知格式时统一回退 7z.exe（覆盖 Scoop 的格式面）；
  ② 消除 7z 原生的静默丢文件（本次实测复现）；③ 依赖成本≈0——`7zip` helper 今天已经被装上了。
- **最"适度"的落点是 PS 兼容层 + 兜底**，而不是替换主力引擎；`7z.dll` 值得作为中长期选项，
  但它的价值在「进度/取消/进程内」而非性能，且必须付出一个 `unsafe` FFI crate + 版本/许可成本。

## 1. 官方 Scoop 侧事实（本地源码 `../Scoop`）

| 维度 | 官方行为 | 出处 |
| --- | --- | --- |
| 引擎选择 | `.zip` → 已装 7zip 或 `USE_EXTERNAL_7ZIP` 时走 7z.exe，否则 PowerShell `Expand-Archive`；`.msi` → lessmsi 或 `msiexec /a`；`.exe`+`innosetup` → innounp；其余按正则命中走 7z.exe | `lib/decompress.ps1:25-48,65-147,171-227,229-267` |
| 7z 调用形态 | `x <path> -o<dest> -xr!*.nsis -y [-ir!<ExtractDir>\*] [Switches] [-aoa/-aos/-aou]`，日志 `7zip.log`，`tar` 二次解包 | `lib/decompress.ps1:94-131` |
| 依赖注入 | `Test-7zipRequirement` 正则 `\.(001|7z|bz(ip)?2?|gz|img|iso|lzma|lzh|nupkg|rar|tar|t[abgpx]z2?|t?zst|xz)(\.[^\d.]+)?$` 命中 → 装 `7zip`；`innosetup` → `innounp`；已装的 helper 会被剔除 | `lib/depends.ps1:109-127,134-146` |
| USE_EXTERNAL_7ZIP=true | **不装 helper**，改用 PATH 上的 `7z.exe`，找不到直接 abort | `lib/decompress.ps1:85-90` |

即：Scoop 的"解压模块"实质就是 7z.exe（+ lessmsi/innounp/msiexec + 一个 PowerShell zip 兜底）。

## 2. hok 侧现状

- 分派入口 `detect_format`（`crates/libscoop/src/internal/archive.rs:34-69`）→ `extract`
  （`:105-144`）：`7z`→pseudo-7z 嗅探（纯 7z / NSIS / Inno / zip-SFX / **7z.exe 兜底**）、
  `zip`→zip crate 并行、`tar/gz/bz2/xz/zst`→tar 栈、`rar`→rars、`msi`→lessmsi、`lzh|iso`→7z.exe。
- 依赖注入与 Scoop 同构（`package/manifest.rs:1373-1391`）：`.7z` 等 URL 仍会装 `7zip`，
  `innosetup` manifest 仍会装 `innounp`。本机 `$env:SCOOP\apps\{7zip,innounp,lessmsi}` 全部在位，
  而 Rust 侧 **innounp 从未被调用**、`7zip` 只在 `iso/lzh` 与兜底路径使用。
  ⇒ **"外部 7z"的依赖成本今天已经付过了，只是没用上。**
- PS 脚本兼容层 `asset_scripts/decompress.ps1`：`Expand-7zipArchive` 不调 7z.exe，只写
  `[[HOK_EXTRACT]]` 标记、由 Rust 在**整段脚本跑完后**统一解压（`:15-20` + `operations/script.rs:323`
  + `operations/extract.rs:160-187`）。已知偏离：`-Switches` 被忽略；`-ExtractDir` 被实现成
  `extract_to`（落 `dest\subdir`）而非 Scoop 的「只取该子树并上提」；`-Removal` 会在 Rust 读标记前
  把源文件删掉（`Test-Path` 失败即静默跳过）。本机 4 个 bucket 静态统计：1295 个 manifest 调用
  `Expand-7zipArchive`，其中 535 处同行带 `-Removal`、154 处带 `-ExtractDir`（**待端到端复现影响面**）。
- 保真度：全部落盘走 `std::fs::write`（`:414`/`:639`/`:825`），全仓无 `set_modified`/属性处理
  ⇒ **mtime、只读/隐藏属性不保留**（7z.exe 会保留）。

## 3. 实测：hok 原生 vs `7z.exe 26.03`（同机、同输入、release）

环境：Ryzen 7 6800H / 16 逻辑核 / 15.2 GB；hok `main` tip `675b991`；
7z.exe = `$env:SCOOP\shims\7z.exe` → 7-Zip 26.03 x64；`7z x -o -y`（Scoop 等价参数）。
hok 侧经**公开 API** 调用真实路径（`session.archive().extract(...)`，即 CLI 同一条代码路径）。

| 缓存文件 | 真实内容 | hok 原生 | 7z.exe | 倍率 | 文件树一致性 |
| --- | --- | --- | --- | --- | --- |
| `nodejs#26.9.0.7z` (26 MB) | 7z，1938 文件/112 MB | **3.14 s** | 7.64 s | hok 2.4× 快 | ❌ **hok 少 4 个 0 字节文件** |
| `ffmpeg-essentials.7z` (35 MB) | 7z，45 文件/329 MB | 5.53 s | **2.90 s** | 7z 1.9× 快 | ✅ |
| `brave#1.95.104.zip` (250 MB) | zip，175 文件/552 MB | **2.98 s** | 7.07 s | hok 2.4× 快 | ✅ |
| `vscode#1.139.0.7z` (337 MB) | 实为 **ZIP**，2662 文件/1.0 GB | **2.74 s** | 20.23 s | hok 7.4× 快 | ✅ |
| `we-meet#3.46.11.413.7z` (302 MB) | PE（NSIS 形态），5974 文件/884 MB | **14.30 s** | 34.04 s | hok 2.4× 快 | ✅ |
| `WorkBuddy#5.6.2.7z` (531 MB) | PE 且两引擎都只认容器，10 文件 | 2.17 s | **1.64 s** | 7z 1.3× 快 | ✅（同为 10 文件） |

观察：

1. **"hok 慢"的假设不成立**：hok 在 zip、zip-SFX（`.7z` 命名实为 zip，占本机 12 个 `.7z` 缓存中的多数）、
   NSIS 形态 PE 上都明显更快；与 devlog `2026-08-15-nsis-extraction-and-7z-perf.md` §4（git 包纯 Rust 20.3 s vs 7z.exe 29.3 s）一致。
2. 7z 唯一明显赢的 `ffmpeg-essentials` 是「45 个文件 + 单块 302 MiB LZMA」——7-Zip 的
   多线程 LZMA2 解码约 2× 于 `lzma-rust2`（313 MiB：hok 解码 6.33 s vs 7z 全量 2.90 s）。
3. `WorkBuddy` 说明**外部 7z 不是万能兜底**：两者都只吐出 10 个文件（PE 容器），
   真正的安装器解包仍要靠 hok 自己的 nsis/innospect 链。

### 3.1 实测复现的缺陷：7z 原生路径静默丢「0 字节文件」

`nodejs` 包 `7z l -slt` 中 4 个条目 `Size = 0 / Packed Size = 0`（无流），7z.exe 全部落盘，hok 一个没落：

```
node-v26.9.0-win-x64\node_modules\npm\.npmrc
…\node-gyp\gyp\pylib\packaging\py.typed
…\gyp\pylib\gyp\generator\__init__.py
…\smart-buffer\docs\ROADMAP.md
```

机制：`extract_7z_entries_mt` 以 **block 为单位**遍历（`archive.rs:375-421`，
`BlockDecoder::for_each_entries`），无流条目不属于任何 block ⇒ 永远访问不到，也不报错。
zip / tar / NSIS / Inno 路径逐个条目遍历，不受影响。
影响面：`py.typed`（PEP 561 标记）、`__init__.py`、`.npmrc` 这类空标记文件是 node/Python 打包的常客，
漏装不会报错、只会运行期才炸。**这是本次调研最该先修的一项（P0，与是否切 7z 无关）。**

**已修（2026-09-25，`4bf1397`）**：用户拍板「先只修 P0」，于是加了一趟 stream-less 条目物化
（`extract_7z_entries_mt` 块 worker join 之后，`!entry.has_stream()` → 目录 `ensure_dir` / 空文件写空文件，
同源 filter + `strip_dir` + `ParentDir` 检查）。除 4 个空文件外，**目录条目本身也无流**——本机 nodejs
实测 460 个目录条目此前同样不在任何 block 里，只靠写文件隐式建父目录。复验（含目录对拍 7z.exe 26.03）：
nodejs 1934 → **2398 条目**（7z.exe 同为 2398）、ffmpeg-essentials 49 条目，
`only-in-7z=0 / only-in-hok=0 / 类型与大小差异=0`；单测 2 条 + 206 B 夹具
`libscoop/tests/fixtures/7z/empty-entries.7z`，worktree `cargo test -p libscoop` 771 passed，
落地树 `cargo test --workspace` 全绿。

## 4. 覆盖差距（可量化，结果比预期小）

对 4 个 bucket 全部 21,898 个 manifest 的 69,965 条 URL，逐条比对
Scoop `Test-7zipRequirement` 与 hok `detect_format`（含 `#/rename` 片段语义）：

- Scoop 会解压、hok **静默当普通文件拷进 app 目录**的只有 **26 处 URL / 约 13 个包**：
  `*.7z.001` 分卷（13）、`*.tar.xz.asc`（5）、`*.7z.ver`（3）、`.html`（2）、空名（2）。
- 其余差距项（`.img` 3 处等）实际都带 `#/dl.7z` 片段，hok 已能识别。
- `.zst` 在本机桶里 100% 是 `.pkg.tar.zst`（tar），hok 的 tar+zstd 路径正确；裸 `.zst` 才会出错。
- `.zip` 命名的 PE-SFX、分卷 `.partN.rar` 的 hok 侧兜底情况见 §6 选项 C。

⇒ 「格式覆盖」本身只值一个小补丁；外部 7z 的主要价值在**失败兜底**与**保真**，不在覆盖。

> **状态更新（2026-09-25）**：§3.1 的 7z 丢条目已修复并落地 `4bf1397`
> （`fix(libscoop): materialize stream-less 7z entries`）：固定/验证见 §3.1 的追记。

## 5. `7z.dll` 作为后端（用户提议）专项

已核实的事实：

- 本机 `apps\7zip\current`：`7z.exe` 577 KB（内部以 UTF-16 字面量 `7z.dll` **动态加载** DLL）、
  `7z.dll` 1.9 MB、`7z.sfx` 215 KB；`7z.dll` 导出 `CreateObject` / `GetHandlerProperty2`。
  hyperscoop 就是把这**两个文件**一起 `include_bytes!` 再写到 `%TEMP%` 后 shell out
  （`hyperscoop/crates/command/src/install/sevenzip.rs:270-319`）。
  ⇒ **7z.exe 只是 7z.dll 的 CLI 前端**：走 DLL 与走 EXE 的解码内核/格式覆盖完全一致，
  差别只在进程、回调、内存与 I/O 路径。
- 接口形态：`CreateObject(clsid, iid, &out)` + `GetHandlerProperty2` + COM 风格 vtable
  （`IInArchive` / `IArchiveExtractCallback` / `ISetProperties` / `IProgress`），
  无 C ABI 稳定声明头，需手写 vtable 或引入第三方绑定（`sevenzip-sys`、`rust7z` 等，成熟度低）。
- **"共用线程"需要澄清**：7z.dll **不能**接入宿主的线程池/rayon；它按 `ISetProperties("mt", …)`
  自建内部线程。DLL 相对 EXE 的真实收益是：
  ① 同进程，省子进程启动与 PATH/临时目录问题；
  ② `mt` 可调，避免「子进程 + 宿主」双重超订；
  ③ **可协作取消**（回调返回 `S_FALSE`/`E_ABORT`），与 hok 的 `CancellationToken` 能接上
     ——这是 EXE 路径结构性做不到的（见 `.agents/docs/reports/refactor/plan.md:116` 已登记为 gap）；
  ④ 逐文件进度回调，取代解析 `-bsp1` 的 `\r` 百分比；
  ⑤ 流式内存（对比 hok 现在整包 `std::fs::read`）。
- 落地约束：
  - `crates/libscoop/Cargo.toml:101-102` `[lints.rust] unsafe_code = "deny"`
    ⇒ FFI 必须另开 crate（对齐 `hok-shim` 的既有做法），不能塞进 libscoop。
  - 来源：用**已安装的 `7zip`**（hok 今天就会装）零体积成本；自带则 +2.45 MB（两个文件原始大小）
    并承担 LGPL-2.1 + unRAR 限制的再分发义务与第三方二进制哈希清单（hyperscoop 的
    `resources/THIRD_PARTY_BINARIES.md` 就是这个成本）。
  - 加载必须按**绝对路径**（`%SCOOP%\apps\7zip\current\7z.dll`），裸名 `7z.dll` 有 DLL 劫持面；
    架构须与 hok 一致。
- 判断：若目标只是「补兜底 + 修丢文件」，DLL 属于过度工程；若目标是**用成熟引擎替掉/收缩自研解码链**
  （rar/nsis/inno/iso 这些弱项），DLL 才成立，且优于 EXE——但实测性能告诉我们，它换不来速度，
  换的是正确性、进度与取消。

## 6. 选项与代价

| 方案 | 内容 | 收益 | 代价 |
| --- | --- | --- | --- |
| **A 不改** | 维持现状 | 0 | 保留 7z 空文件丢包（P0）+ 13 个分卷包静默错装 |
| **B 全切 7z.exe** | 与 Scoop 等价，全部走外部 7z | 覆盖/保真对齐 Scoop | 实测多数场景**更慢**；丢纯 Rust 属性（CI 无 7zip 也能测、无版本漂移、无外部依赖）；PS 层/脚本层大改 |
| **C 适度混合（推荐）** | ① 修 7z 空文件；② 所有原生路径加「失败 → 7z.exe 单次兜底」（zip/tar/rar/msi 目前无兜底）；③ 未知格式按 Scoop `Test-7zipRequirement` 兜底（补 `.001`/`.img` 等）；④ PS 兼容层 `Expand-7zipArchive/-MsiArchive` 改为**直接调 7z.exe/lessmsi**（Scoop 语义，顺带修 `-Switches`/`-ExtractDir`/`-Removal`）；⑤ 纯 Rust 保留为默认快路 | 覆盖 + 兜底 + 保真，性能不倒退，依赖不新增（7zip 已装） | 一个外部工具失败模式（必须有「缺 7z 就回退纯 Rust/明确报错」的判定）；多一条代码路径要测 |
| **D 7z.dll 后端** | 新 `crates/hok-7z` FFI crate，用已装 `7z.dll`；先只覆盖弱项格式（rar/iso/NSIS/Inno），纯 Rust 保留 | 进程内、可取消、可限线程、格式覆盖成熟 | unsafe FFI crate + vtable 维护；版本/架构/许可矩阵；DLL 加载路径安全要求；收益不含速度 |

## 7. 待确认（一次性问清）

**已定（2026-09-25 用户拍板）**：只先落地 P0 空文件/空目录修复（= 方案 C 的第 ① 项），其余不动
——「目前模式貌似没有太大问题」。已落地 `4bf1397`。仍未拍板的三项：

1. 走 **C 的剩余项**（兜底 + PS 层切外部 7z），还是先只保留 P0 修复？
2. 「原生失败即回退 7z.exe」是否作为默认语义（保持纯 Rust 快路）？
3. `7z.dll`（方案 D）现在做可行性 spike，还是只登记为后续选项？
4. PS 兼容层是否允许直接改调外部工具（会改变 535 处 `-Removal` / 154 处 `-ExtractDir` 的实际行为，
   需要先端到端复现这些包的现状）。

## 附：复现方式

- 7z.exe 基线：`7z x <cache 文件> -o<dest> -y -bso0 -bsp0`（脚本 `D:\hok-bench\bench7z.ps1`）。
- hok 原生路径：临时 crate `D:\hok-bench\extract-bench`，只用公开 API
  `Session::new().archive().extract(...)`，`CARGO_TARGET_DIR=<hok>/target` 复用产物；
  树比对脚本 `D:\hok-bench\compare.ps1`（路径 + 大小集合差集）。
- 覆盖差距统计：`D:\hok-bench\gap.ps1`（Scoop 正则 vs hok `detect_format`，含 `#` 片段语义）。
- 以上临时目录均已清理解压产物；脚本保留可复跑。
