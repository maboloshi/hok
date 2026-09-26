# update verify/extract 缓存文件名不一致 + zip 反斜杠解压修复

> 2026-08-21 ｜ 2 commit：`fdd1114` `9e91b31`

**用户报告**（另一台设备 `hok update` 输出）：大量 `failed to verify 'xxx': 系统找不到指定的文件 (os error 2)`、croc 解压 `cannot write ...\src\codephrase\: 系统找不到指定的路径 (os error 3)`、potplayer `Hash mismatch`、以及"疑似重复下载"。

## 已确认问题 1：verify/extract 对 upgradable 缓存文件名处理不一致（`fdd1114`）
**根因**：`load_cache`（`download.rs`）与下载器算缓存文件名用 `pkg.upgradable().unwrap_or(pkg)`，但 `verify_downloads`（`download_verify.rs`）和 commit 阶段的 `extract_archives`/`copy_downloaded_files`（`operations/extract.rs`）直接用 `pkg.download_filenames()`。三处对 upgradable 处理不一致——force/reinstall 模式 `set.packages` 装的是**已安装旧版本包**（其 `upgradable` 指向 bucket 新版本）→ `load_cache` 用**新版本**文件名缓存命中，`verify` 却找**旧版本**文件名 → 缓存里没有 → "文件不存在"。
**修复**：三处统一 `pkg.upgradable().unwrap_or(pkg)`，让验证/解压检查的缓存文件名与下载器产出完全一致（与官方 Scoop `check_hash` 验证下载写入的同一缓存文件语义对齐）。
**日志铁证**：`正在从缓存加载 manggo#0.7.9#ca39fcf.7z...`（load_cache 用 0.7.9 命中）而本地 bucket 是 0.7.8 → verify 找 0.7.8 文件不存在；且所有 verify 失败的包都是缓存命中，真正下载的包全部通过。

## 已确认问题 2：zip 条目用反斜杠分隔符解压失败（`9e91b31`）
**根因**：croc 发布 zip 条目名用 `\` 分隔（实测缓存 `croc#11.2.1#128f67d.zip`：`src\codephrase\`、`src\codephrase\wordlists\LICENSE.txt`）。`extract_zip_mt` 只跳过以 `/` 结尾的目录条目 → `src\codephrase\` 以 `\` 结尾未被识别为目录，被当文件写入 `...\src\codephrase\`（以 `\` 结尾的非法路径）→ Windows `ERROR_PATH_NOT_FOUND`。
**修复**：解压时把 zip 条目名 `\` 规范化为 `/`；新增回归测试 `test_extract_zip_backslash_separators` 直接复现 croc 布局。

## 非代码问题（非本次范围）
- **potplayer `Hash mismatch`**：daumcdn 的 manifest hash 过期 / 下载被代理污染，属数据问题，hok 正确检测并删缓存；
- **"疑似重复下载"**：日志里同一包进度条重复出现（bun 出现 6 次）是 indicatif `MultiProgress` 在**非 TTY/管道输出**下每次重绘整组 bar 的显示问题，`download()` 每个文件只下载一次，非真实重复下载。

## 教训（本次排查耗时复盘）
1. **现象先归类，按「可证伪成本」排序**：日志 4 类症状应先做能最快出硬证据的（解压失败→看真实 zip 条目；verify 失败→对比两处文件名），而非顺着日志读源码；
2. **先看产物/缓存，后读源码**：croc 反斜杠实际看 zip 条目几分钟就实锤，纯源码推理反而绕远；
3. **用最小实验终结推理循环**：对 upgradable 不一致这类"两处应一致但表现不一致"，直接写单测对比 `download_filenames` 或对照本地官方 Scoop 源码，一次实验顶十次读码；
4. **关键大文件一次读完**：`archive.rs` 输出被截断分两次读，浪费轮次；
5. **动手前先确认输入可信**：日志是"另一台设备"输出，应先从日志自身推断证据，不在错误基线（本机缓存）上找证据。

## 提交与签名
- 两 commit 按主题拆分（upgradable 一致性 / zip 反斜杠）分别提交；
- GPG 签名遇 `Win32 error 5`：根因是 MSYS gpg 在 `HOME` 为空时把中文用户名路径乱码找不到 `.gnupg`；解决 `$env:HOME=$env:USERPROFILE` 后重签名（`rebase --exec "git commit --amend --no-edit -S" HEAD~2`）。签名方式详见技能 `.agents/skills/hok-gpg-signing.md`。


---
