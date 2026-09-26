---
name: hok-audit-fix-history
title: Hok 审计修复史（三轮）+ uninstall 对齐收官
description: hok 三轮审计修复史与 uninstall 对齐收官；防踩坑：事件结束必发 Done、update -f 用 Force、UserAborted 配套。
metadata:
  type: project
---

Hok 审计修复史（2026-08-12/13，全部已提交）+ uninstall 对齐收官。用户驱动：三轮 explore 审计 + review 复核，每轮用户选「高+中+低全部」修复。按轮分组（避免原三条命名混淆：historical 的「两轮」是重构残留轮，非 second-round）。

**第 1 轮 · 历史重构残留**（旧大重构+小修小补遗留）：
- URL 文件名两语义：`internal::url::url_filename`（fragment `#/dl.7z` 参与 → "dl.7z"，Split-Path 语义）vs `url_remote_filename`（原始名 "foo.exe"）——**checkhashes 缓存命名用 remote，installer fallback 用 filename**；create.rs 勿统一（尾斜杠 fallback "download" 被测试锁）
- 进程枚举 fail-closed（running_processes_under 失败不再当无进程）
- Manifest.hash 死字段删除（假注释 cache validation 从未读取）
- Cmd trait 脚手架删除（36 个零调用 impl；组 import 删 `, Cmd` 保留 validate_dir/ensure_global）
- 死函数清理（os_is_arch64/scoop_arch/buckets_dir/walkdir_files/sysinfo 注释块）
- **审计误判保留**：manifest_cache::refresh 有调用者、dag 未用方法有测试、auto_pr split 非 bug

**第 2 轮 · 隐藏问题**（用户实测撞出）：
- **非交互卡死全家桶**：prompt_yes_no EOF/空行 → 保守 false；uninstall 补 -y；空 remove 事务不弹确认；候选选择尊重 AssumeYes（内置选排序后第一个）；**候选循环 EOF → break 0**
- download 失败 exit 非零（先发 DownloadDone 再报错；**空包/错误路径也必须发 DownloadDone**，否则 handle.join() 挂死——空包死锁是第 3 轮发现的漏网）
- running-check 提前 + 过滤后重建 PackageSet（运行中包不再下载）
- 事件闭环：ResolveDone/SizingDone 补发；outdated 文案归 update 命令层（事件循环 PackageSyncDone 只 break）
- ignore_failures 默认 false（对齐 Scoop）；manifest_cache 序列化失败跳过条目
- 光标恢复（exit 前 Show）；桶枚举失败 warn 不误报 PackageNotFound

**第 3 轮 · 修复回归边界**：
- download 空包死锁（packages.is_empty 也发 DownloadDone）
- **update -f 语义**：`SyncOption::Force`（保留 OnlyUpgrade 已安装限制 + 重装当前版本）；注意 **upgradable_version filter 在 Force 时不能滤掉包**（曾致 force no-op，有回归测试 only_upgrade_force_reinstalls_current_version）
- 预查不误判缺失包（missing_target fall through 报 PackageNotFound）
- **UserAborted**（confirm 拒绝返回它，exit 0 不打 ok_all）；**关键：sync.rs 错误路径也发结束事件**（否则事件循环 break 不了 join 卡死）
- 低项：reinstall ok_all、cui 死参、shim 排序无 panic、validate_dir 失败 exit 非零

## uninstall 对齐收官（2026-08，已提交）官方参照 `../Scoop\libexec\scoop-uninstall.ps1`（本地副本）：
1. `unlink_current` 位置：persist 之后移到 shim::remove/shortcut_remove 之后、psmodule::remove/env::remove 之前（官方 `$refdir = unlink_current $dir` 位置）
2. `post_uninstall` 时机：移到删版本目录之后，包进 `if let Some(version_dir)` 块（官方 `if ($version)` 结构）；版本目录在 unlink_current **之前**解析（select_current_version 依赖 current\manifest.json）
3. 删除粒度：`remove_dir(app_dir)` 一次删整个 → ①persist_unlink+remove_dir(version_dir) ②遍历 app_dir 下非 current 目录逐个删（官方 old-version loop）③删残留 current 链接 ④仅当 app_dir 空时删它；目录不存在继续、被占用返回 Error::Custom（对应官方 continue）
4. `persist::unlink(session, package, version_dir)` 改为按版本目录解除（官方 unlink_persist_data $manifest $dir）
5. `unlink_current` 容忍 current 不存在（NotFound 静默，对齐官方 Test-Path 保护/NO_JUNCTION）
- 残余 nit（未改）：check_not_running 在 pre_uninstall 之前（官方先 hook 后 test）；pre_uninstall/uninstaller/shim/shortcut 无条件运行（官方在 `if ($version)` 内）；post_uninstall working_dir 是 &app_dir（官方 $refdir）；persist.rs::unlink 的 session 参数未用

**通用防踩坑**：
- 事件循环：任何流程结束/错误都必须发对应 Done 事件（PackageSyncDone/DownloadDone），否则 CLI handle.join() 挂死
- sync 语义改动先查 SyncOption 组合（Force/OnlyUpgrade/AssumeYes）
- rustfmt 单文件会递归格式化子模块（package/mod.rs → manifest.rs 既有空行被误改）——提交前 checkout 恢复
- 同文件多主题无法拆 commit 时 message 注明
- 历史相关：审计修复后的对标进度见 [[scoop-gap-audit]]，shim/shortcut 专题见 [[hok-shim-dual-variant]]

## $fname 变量修复（2026-08-15，commit 2207a8c）
- **问题**：installer.script 执行环境缺少 `$fname`（Scoop 语义：第一个下载 URL 的文件名，install.ps1 `$Name[0]`）。大量 manifest 用 `Expand-InnoArchive "$dir\$fname"`（zed 等）——hok 里 `$fname` 为空 → 路径变 `$dir\`（目录）→ innounp exit 3（"Cannot open ... \"，日志在 apps\<app>\innounp.log，GBK 乱码"系统找不到指定的文件"）
- **根因链**：copy_downloaded_files（extract.rs）把安装器以 URL 文件名复制到 $dir（如 Zed-x86_64.exe），但 run_script preamble（script.rs）只定义 $dir/$original_dir/$scoopdir/$bucketsdir/$persist_dir/$version/$app/$bucket/$architecture/$global/$cmd——**漏了 $fname**
- **修复**：run_script 注入 `$fname = '<url 文件名>'`（`first_download_fname`：strip_url_query + rsplit('/')，与 copy 目标名一致；单引号转义）；`expand_scoop_str`（installer.file args）**不需要** $fname（Scoop substitute 只展开 $dir/$version/$global）
- **验证**：`hok update zed`（1.11.3→1.15.0，缓存 92MB Inno 安装器）端到端成功——Expand-InnoArchive 解压、shim/快捷方式创建、bin\zed.exe 存在；单元测试 test_first_download_fname
- 注意：PowerShell 传含 `$` 的 commit message 用**单引号**（双引号里 `$fname` 被展开为空——已踩，amend 修正）

## running 过滤前置到 transaction 组装前（2026-08-15，commit 57e23ae）
- **问题**：`hok update *` 且 `ignore-failures=true` 时，运行中 app 警告"skip updating"但**确认列表仍显示全部包**（含被 skip 的）——列表与实际行为矛盾。根因：running 检查（sync_install.rs 原 266-283）在 `transaction.set_upgrade()` 之后才过滤 `packages`（下载/执行集合），而 `Transaction.upgrade`（OnceCell，只写一次）仍是全部 → confirm_transaction 显示未过滤列表
- **修复**：过滤提前——`filter_not_running(session, upgradable, replaceable, ignore_failure)` 在 transaction 组装（set_upgrade/set_replace）**之前**执行，删除原过滤块；installable 不过滤（新装无 apps/<name> 目录，check_not_running 恒 NotRunning）
- **语义保持**：`ignore_failures` → 警告+跳过；否则 `Err(AppRunning)` 整体 abort；`ignore_running_processes` 配置 → 警告+继续（RunningCheck::Ignored 保留）（**注**：这条 2026-08-15 的口径在 `a4ea0b7` 被改成"两支都跳过"，2026-09-13 晚已回正为"默认跳过 / 开键继续"，详见 `.agents/skills/scoop-gap-audit.md` 同名条）
- **验证**：启动 node 进程（apps\nodejs\current\node.exe）→ `update *` → 确认列表只剩 pixpin（nodejs/PowerShell/git/directoryopus-cn 正确排除）；用户实测通过
- 教训：OnceCell 型 transaction 组装后不可再改——**前置过滤**优于事后同步

## 解压进度条：覆写残留 + 性能（2026-08-15，commit 8b408c3）
- **问题 1（覆写残留）**：scoop_handler `print!("\r ...")` 覆写不清行——短行覆盖长行时尾部残留旧字符（git 9567 文件出现 `(679/9567)67)7)/9567)67)67)567))7)`）
- **问题 2（性能）**：archive.rs 各 extract 循环**每文件**发 `PackageExtractProgress`（git 9567 次）→ 事件通道 + stdout flush 开销大
- **修复**：①渲染端 `print!("\r\x1b[2K  ...")`（先清整行再写）；②发送端节流 `emit_extract_progress(emitter, i, total, name)`——**每 50 个 + 最后一个**（7z/zip 带计数；tar/rar/innosetup/nsis 无 total 只节流）
- **实测基准**（#[ignore] bench_progress_overhead，10000 文件 zip，真实 print+flush sink）：无进度 **18.83s** vs 节流进度 **19.63s**——**开销 ~4.2%**；修复前逐文件渲染估算 25-50%（Windows 控制台 flush 每次 ~0.5-1ms）
- 教训：进度渲染开销主要在 **stdout flush**（非通道）；交互式进度条应降频（每 N 或定时）而非逐项

## 7z 提取性能：read_file 灾难 → 块并行（2026-08-15，commit a92f9e2）
- **根因 1（>10min）**：`extract_7z_entries` 用 `ArchiveReader::read_file(name)` 逐文件——sevenz-rust2 文档明言 solid 归档下"needs to decode all data before the actual file"（reader.rs:1463），git 包（56MB/6 solid 块/9567 文件）每次重新解压块前缀 → O(n×块) 灾难
- **修复 1**：改 `for_each_entries`（每块一次解压）→ 63s（7z.exe 29s）
- **瓶颈拆分（decode-only 实测）**：解码 41.7s（66%）+ IO 21s（34%）——解码是瓶颈（lzma-rust2 纯 Rust LZMA 单线程）
- **修复 2（块并行）**：`extract_7z_entries_mt`——每块一个 worker（`BlockDecoder`，上限 available_parallelism），worker 池原子取块；extract_7z 统一读内存 + magic 定位（SFX 校验 `Archive::read`）+ `owned_filter`/`PathBuf`/`Option<Sender>` Arc 共享（thread::spawn 需 'static）
- **结果**：63 → **39.2s**（1.6x）；剩余差距 = 最大块单线程解码（LZMA1 无块内 MT，sevenz 仅 LZMA2 支持）+ 块大小不均；7z.exe 29s 用 C liblzma——纯 Rust 已到上限，不换 xz2（非纯 Rust）
- **配套**：进度显示改为 `包名 n/total` 计数（去文件名）；emit_extract_progress 无 total 不发
- 教训：7z 大包优先 `for_each_entries`/`BlockDecoder`（块级），**禁 `read_file` 逐文件**（solid 灾难）；thread::spawn 闭包需 'static——Arc 共享 data/filter/dest

## 7z 性能最终结论：debug 假象，release 已优于 7z（2026-08-15）
- **关键教训：所有基准必须 `--release`！** 之前 37s 等全是 `cargo test`（debug）数字——lzma-rust2 的 `optimization` feature（unsafe 汇编）只在 release 生效，debug 慢 3-5 倍
- **release 实测（git 包 56MB/6 solid 块/9567 文件）**：串行解码 11.1s、并行解码 9.0s、**全量提取 20.3s——优于 7z.exe（29.3s）44%**
- **xz2 实验（C liblzma）**：同一 LZMA1 流 lzma-rust2 0.677s vs xz2 0.790s——**lzma-rust2 快 14%**（与文档一致），换 xz2 反而更慢，放弃
- **filter 链（BCJ2/Delta）无差距**：21MB 纯 LZMA vs BCJ2+LZMA 测试归档，debug 下均 ~2.8s（release 会快），无 BCJ2 特有开销
- **依赖升级**：sevenz-rust2 0.21.4 + lzma-rust2 0.18.1（0c05848）无性能变化，仅版本更新
- 结论：**纯 Rust 解压已是最优，无需再优化**；bench_git_7z_extract（块分布 + decode-only + 并行上限诊断）保留，用 `cargo test --release` 跑
