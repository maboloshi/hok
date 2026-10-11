# `hok cleanup` 并行化 + 进度呈现规格（2026-09-20 定稿，待开工）

> **状态**：**已实现并落地**（2026-09-20；4 提交 `3bca476` `54a87d0` `2ad6e9d` `e8c09f1` 线性 `--ff-only` 进 `refactor/edition-2024`，无 merge commit）。
> 实现与本文模拟的两处外观差异：① 失败行沿用 `cui::RowUI::fail` 的既有形状 `✗ app：ver (err)`（ASCII 括号，非本文模拟的全角括号）；② 管道模式的成功行带 `output::done` 的 2 空格缩进。
> **已定**：不做 rename、同步删除到底、spinner 呈现过程、K1 = C1、K2 = 0；K3/K4/K5 暂按推荐（可推翻）。
> **相关实测**：`.agents/docs/research/cleanup-delete-perf.md`（删除耗时、占用语义、复跑脚本）。
> **一句话**：删除仍同步做完（3 万文件 ≈8 s，4 线程），每个旧版本一行 spinner 实时呈现；
> 失败就是**该行原地收尾成 `✗`**，末尾一条 `WARN` 汇总，退出码 `0`。

## 1. 范围

| 项 | 决定 |
|---|---|
| rename / staging / 延迟删除 | **不做**（用户 2026-09-20 决定） |
| 同步 or 后台 | **同步删除到底**，命令返回时删除已完成 |
| 并行 | **做**，单位 = 旧版本目录，动态队列，`std::thread::scope`（无新依赖） |
| 并发度 | `min(4, available_parallelism)`（实测 4 线程 4.2–4.4×） |
| 进度呈现 | indicatif spinner（复用 `cui::RowUI`） |

## 2. 最终规格

| 维度 | 规格 |
|---|---|
| 进度行 | 每个**旧版本目录**一行 spinner；最多 `并发度` 行在转，完成即**原地收尾**为永久行 |
| 成功行 | `✓ app1：0.9.0`（版本号 green） |
| 失败行 | `✗ app4：0.8.0（拒绝访问。 (os error 32)）` —— **就是原来在转的那一行**（K1 = C1） |
| 错误层级 | 只有两层：失败行 + 末尾汇总；**不再**有「已清理 N 个旧版本，M 个失败…」那层 |
| 汇总 | 仅**有失败**时：`WARN N 个应用的旧版本未能移除：a, b（关闭占用程序后重试；这些目录已被部分删除）`（stderr） |
| 收尾行 | `✓ 一切正常！` 仅**零失败**且 `-a` 时打印（今天是无条件，矛盾点） |
| 指定 app 无旧版本 | `没有旧版本需要清理。`（dim）；`-v` 时 `✓ app1 已清理干净` |
| 退出码 | `0`（K2 = 0，与 hok `update` 的 WARN 口径一致） |
| 顺序 | TTY：按完成顺序收尾；管道：按**输入顺序**缓冲输出（可复现） |
| 管道模式 | 不画 spinner（indicatif 非 TTY 什么都不画），同一文本按行输出（K3 = 与 TTY 一致）；成功/信息 → stdout，失败/汇总 → stderr |
| 收尾行缩进 | 期望列 0 起（K4）；**实测未实现**——收尾行仍带 2 空格缩进，见 §3.9 |
| `-g` 阶段头 | 不加（K5，与官方一致）；全局批的失败已在汇总行里点名 |
| lib 职责 | `CleanupResult` 携带失败明细（ver + msg）；**lib 不再自己打印**，渲染全在 CLI |
| verbosity | 失败信息任何 verbosity 都显示；`-v` 额外显示「已清理干净」 |

## 3. 模拟输出全集

（字符串取自现有 i18n；数字按实测 3 万文件 / 4 线程 ≈8 s。**未实现**。）

### 3.1 TTY `hok cleanup -a` 全成功

t≈0.4 s：
```
  ⠸ app4：移除 0.8.0 ...
  ⠸ app7：移除 0.9.0 ...
  ⠸ app9：移除 1.0.0 ...
  ⠸ app11：移除 0.9.0 ...
```
t≈4.0 s（完成的原地收尾，位子让给下一个）：
```
  ✓ app1：0.9.0 0.8.0
  ✓ app2：0.9.0
  ✓ app3：0.9.0
  ⠙ app4：移除 0.8.0 ...
  ⠹ app7：移除 0.9.0 ...
```
结束：
```
$ time hok cleanup -a
  ✓ app1：0.9.0 0.8.0
  ✓ app2：0.9.0
  ...
  ✓ app12：0.9.0
  ✓ 一切正常！

real  0m8.2s
```

### 3.2 TTY `hok cleanup -a` 有失败（K1 = C1）

```
$ hok cleanup -a
  ✓ app1：0.9.0 0.8.0
  ✓ app2：0.9.0
  ✓ app4：0.9.0
  ✗ app4：0.8.0（拒绝访问。 (os error 5)）
  ✗ app9：1.0.0（另一个程序正在使用此文件，进程无法访问。 (os error 32)）
  ✓ app7：0.9.0
  ...
  ✓ app12：0.9.0
  WARN 2 个应用的旧版本未能移除：app4, app9（关闭占用程序后重试；这些目录已被部分删除）

$ echo $?
0
```
说明：失败行 = 该版本自己的 spinner 行原地收尾（不新增行、不与活行交错、天然与 app/版本绑定）；
成功的版本仍有 `✓` 行（部分成功可见）；`✓ 一切正常！` 因有失败而不出现。

> **已否决的备选（C2，每 app 一行）**：`✗ app4：0.9.0 已移除；0.8.0 失败（拒绝访问。 (os error 5)）`。
> 否决理由：同 app 多版本失败时会丢掉各自的 os error，且与「失败行 = 原地收尾的那一行」不一致。

### 3.3 TTY 指定单 app

```
$ hok cleanup app1                 # 成功
  ⠸ app1：移除 0.9.0 ...
  ✓ app1：0.9.0 0.8.0

$ hok cleanup app9                 # 失败
  ⠸ app9：移除 1.0.0 ...
  ✗ app9：1.0.0（另一个程序正在使用此文件，进程无法访问。 (os error 32)）
  WARN 1 个应用的旧版本未能移除：app9（关闭占用程序后重试；该目录已被部分删除）
  exit 0

$ hok cleanup app1                 # 没有旧版本
  没有旧版本需要清理。

$ hok cleanup -v app1              # 没有旧版本 + verbose
  ✓ app1 已清理干净
```
（指定 app 形式**不**打「一切正常！」，与今天 `src/cmd/cleanup.rs:116` 一致。）

### 3.4 TTY `hok cleanup -a -g`（本地批 + 全局批）

```
$ hok cleanup -a -g
  ✓ app1：0.9.0
  ...
  ✓ app12：0.9.0
  ✓ appA：1.0.0
  ✓ 一切正常！
```
K5 = 不加「全局」阶段头 ⇒ 两批的行连在一起（与今天 `src/cmd/cleanup.rs:60-72` 的两段式一致）。

### 3.5 管道 / CI

```
$ hok cleanup -a > out.log 2> err.log
$ cat out.log
  ✓ app1：0.9.0 0.8.0
  ✓ app2：0.9.0
  ...
  ✓ app12：0.9.0
$ cat err.log
  ✗ app4：0.8.0（拒绝访问。 (os error 5)）
  ✗ app9：1.0.0（另一个程序正在使用此文件，进程无法访问。 (os error 32)）
WARN 2 个应用的旧版本未能移除：app4, app9（关闭占用程序后重试；这些目录已被部分删除）
```
按输入顺序输出 ⇒ 同一棵树两次运行的日志逐字节相同（除 os error 文本）。

### 3.6 spinner 之前：未安装 / 损坏 / 跨 scope（**保持现状**）

```
$ hok cleanup -a ghost
  ERROR ghost 未安装。
  ⠸ app1：移除 0.9.0 ...
  ✓ app1：0.9.0 0.8.0
  ✓ 一切正常！
```
由 `confirm_installation_status`（`crates/libscoop/src/package/cleanup.rs:75-123`）在起行**之前**发出，与 spinner 无冲突，不改。

### 3.7 退出码

| 场景 | 退出码 |
|---|---|
| 全部成功 | 0 |
| 部分/全部失败 | **0**（K2）+ `WARN` 汇总 |
| 指定 app 未安装 / 跨 scope | 1（现状，发生在确认阶段） |
| `--global` 非提升 | 1（`ensure_global`，现状） |

### 3.8 与官方 Scoop 对照

官方（`Scoop/libexec/scoop-cleanup.ps1:43-62`，串行；下面报错块为真实 PS 5.1 输出）：
```
> scoop cleanup -a
Removing app1: 0.9.0 0.8.0
Removing app2: 0.9.0
Removing app9: 1.0.0
Remove-Item : Cannot remove item ...\apps\app9\1.0.0\f3.bin: The process cannot access
 the file 'f3.bin' because it is being used by another process.
At ...\scoop-cleanup.ps1:50 char:9
+         Remove-Item $dir -ErrorAction Stop -Recurse -Force
+         ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~
    + CategoryInfo          : WriteError: (f3.bin:FileInfo) [Remove-Item], IOException
    + FullyQualifiedErrorId : RemoveFileSystemItemIOError,Microsoft.PowerShell.Commands.RemoveItemCommand
> echo $LASTEXITCODE
1
```

| 维度 | 官方 Scoop | hok（本规格） |
|---|---|---|
| 进度 | 无 spinner，`Removing appN:` 黄色 + 版本逐个 append | spinner 每版本一行，完成原地收尾 |
| 并发 | 串行 | `min(4, 核数)` |
| 版本顺序 | `Get-ChildItem` 原序（不可复现） | newest-first（可复现） |
| 失败呈现 | PowerShell 原始 `IOException` 块 | 一行 `✗ app：ver（原因）` |
| 失败后续 | **中断整批**，剩下 app 不清理 | 继续清理其余 app + 末尾 `WARN` 汇总 |
| 部分成功 | 不区分（直接抛） | `✓`/`✗` 分行 |
| 收尾行 | `Everything is shiny now!`（失败时根本不出现） | `✓ 一切正常！`（仅零失败，语义一致） |

### 3.9 TTY 实测（2026-09-20，ConPTY 捕获 + 回放）

真实控制台（ConPTY，100×20）跑 `hok cleanup -a`（4 个 app × 4000 文件旧版本，`foxtrot` 的旧版本里有个被占用文件）。

运行中（4 行同时在转 = 4 个 worker）：
```
⠙ 正在移除 alpha： 0.9.0 ...
⠹ 正在移除 bravo： 0.9.0 ...
⠙ 正在移除 charlie： 0.9.0 ...
⠙ 正在移除 delta： 0.9.0 ...
```
全部成功终帧：
```
✓ alpha：0.9.0
✓ bravo：0.9.0
✓ charlie：0.9.0
✓ delta：0.9.0
  ✓ 一切正常！
```
有失败终帧（`foxtrot` 被占用）：
```
✓ alpha：0.9.0
✓ bravo：0.9.0
✓ charlie：0.9.0
✗ foxtrot：0.9.0 (另一个程序正在使用此文件，进程无法访问。 (os error 32))
WARN 1 个应用的旧版本未能移除：foxtrot（请关闭占用这些文件的程序后重试；相关目录可能已被部分删除）
```
（颜色：spinner 青、`✓`/版本绿、`✗`/错误红、`WARN` 黄；上面是去色文本。）

**唯一外观瑕疵（K4 未实现）**：收尾行 `✓ 一切正常！` 走 `output::done`，带 2 空格缩进，而 indicatif 的版本行从列 0 起 ⇒ 比行内容右移两格。修法二选一：收尾行也走行渲染（列 0），或给每行也加 2 空格。

**捕获工具链**（本机实测可用，均在 `.agents/scripts/cleanup-bench/`）：
- `conpty-capture.rs`：GUI 子系统启动器（`#![windows_subsystem = "windows"]`）+ ConPTY，把真终端收到的 VT 流写文件。**必须是无控制台的 GUI 子系统宿主**——普通控制台宿主会把自己的 std 句柄交给子进程，子进程于是用宿主管道而不是伪控制台（实测子进程报 `[Console]::IsOutputRedirected=True`、输出漏回宿主）；另外 `CreateProcessW` 的 `DllImport` 必须 `CharSet = CharSet.Unicode`（默认 ANSI 编组会把命令行变成乱码 → `CreateProcessW err 2`）。
- `vt-screen.js`：迷你 VT 模拟器，把捕获回放成屏幕帧。**必须实现 CJK 双宽 + 终端自动折行**（ConPTY 每行补空格到列宽、靠折行换行；把字符数当列数会得到错行）。
- 样例捕获：`sample-tty-cleanup.bin`（含失败）、`sample-tty-cleanup-ok.bin`（全成功）。

## 4. 决策记录

| # | 问题 | 决定 | 日期 |
|---|---|---|---|
| **K1** | 失败行粒度 | **C1**：每个失败版本一行 `✗ app：ver（原因）` | 2026-09-20 |
| **K2** | 失败时退出码 | **`0`** + `WARN` 汇总（与 hok `update` 口径一致） | 2026-09-20 |
| **K3** | 管道模式粒度 | 与 TTY 一致（`✓ app：ver` 逐行）—— 暂按推荐，可推翻 | 待用户点头 |
| **K4** | 收尾行缩进 | 期望列 0 起 —— **未实现**：实测收尾行仍是 2 空格缩进（见 §3.9），要么改收尾行、要么给行加缩进 | 待用户决定 |
| **K5** | `-g` 阶段头 | 不加 —— 暂按推荐，可推翻 | 待用户点头 |

## 5. 改动面

| 文件 | 改动 |
|---|---|
| `crates/libscoop/src/package/cleanup.rs` | `CleanupResult.failed: usize` → 携带失败明细（`Vec<(ver, msg)>`）；`cleanup()` 内并行（`std::thread::scope`，结果**按输入顺序**返回）；**删除** `session.output().error(...)` 调用；单测随结果结构更新 |
| `src/cmd/cleanup.rs` | 渲染改为 `cui::RowUI`（TTY）/ 纯行（管道）双模；失败行原地收尾（行 id = `app：ver`）；末尾 `WARN` 汇总；`✓ 一切正常！` 加零失败条件；退出码保持 0 |
| `src/cui.rs` | 预计零改动（`start`/`set_spin`/`done_with`/`fail`/`finish_all` 已够用） |
| `locales/{zh,en}.yml` | 废弃 `cleanup_removed_failed`（「…（重启后可重试）」）；新增失败汇总键、成功行标签键；`cleanup_remove_failed` 补「请关闭占用该应用的程序后重试」 |
| `tests/`、单测 | 新增：并行下结果顺序稳定、失败明细、管道输出与退出码；既有 cleanup 单测随 `CleanupResult` 结构更新 |
| 依赖 | **无新增**（`std::thread::scope` + 既有 `indicatif`） |

## 6. 风险 / 取舍

1. **删除不可逆**：并行前需对每个 (app, version) 单元做校验（`current` 版本与 `_*.old*` 永不入选），保留 `cleanup_no_junction_ignores_stale_current` 这类回归测试；
2. **半删除是既有事实**（失败时目录里已删掉一部分文件），本规格不改这一点，只把它**说清楚**；
3. **并发度固定 4** 而非按核数无上限：删除是重 IO，实测 4 线程已 4.2–4.4×，再高收益不明且放大误删半径；将来可加 `--jobs`；
4. **不动 rename** ⇒ 空间当场回收、零新状态，但「运行中的程序」仍需用户关掉后重跑（实测 rename 是唯一能绕开的路径，已明确放弃）。
