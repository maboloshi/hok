# hok 输出系统综合方案

> 状态：设计已定稿，待分阶段实施。来源：综合官方 Scoop、pacman、hyperscoop、BucketUpdateUI 现有实现的设计讨论（2026-08）。

## 一、设计原则
1. 先计划、后执行：事务摘要 + 确认闸门先于执行段（pacman）。
2. 一行一个可行动的语义：每行回答「哪个包 + 什么动作 + 结果」，无纯状态行。
3. 单渲染器、单风格：不保留 scoop/pacman 双路径。
4. 计数器 + 逐包状态机：`(k/N)` + 逐行闭合，乱序并行也秩序井然。
5. TTY/管道双通道：TTY 有转圈/进度条，非 TTY 塌缩为可 grep 的计数行。
6. 颜色语义化、可关：单一调色板（`src/output.rs::style`），`NO_COLOR`/`--no-color`/config 关闭。
7. 漂亮输出恒最小；详细颗粒度归日志排错。

## 二、渲染架构
```
libscoop 事件 ──► 统一渲染器 render(semantic_event, verbosity, tty)
                        ├─ 漂亮输出（stdout）：最小，恒定
                        └─ 日志通道（tracing）：-v/-vv/-vvv 驱动
```
砍掉：`output-style` 双风格、`--detail`、`DETAIL_MODE`、`output::detail()`。

## 三、彩色规范（语义表）

单一调色板 `src/output.rs::style`：每个语义一个角色，命令不再直接调 crossterm `Stylize`。
只用**普通亮度** ANSI 色（36/32/33/31/90）+ 默认前景加粗——深浅主题都清晰。

| 语义 | 角色 / ANSI | 用于 |
|---|---|---|
| 阶段头、标签、箭头 | accent `dark_cyan` (36) | :: resolving…、字段名、`->`、进度条填充 |
| 包名 / 实体名 | emphasis 粗体 + 默认前景 | 摘要、计数行、表格名称列 |
| 版本、成功 ✓、ok | success `dark_green` (32) | foo-1.0、包闭合 |
| 警告 | warning `dark_yellow` (33) | 冲突、hold、建议 |
| 失败 ✗、错误 | danger `dark_red` (31) | ERROR、失败、需关注词 |
| 计数、注释、分隔线 | dim `dark_grey` (90) | (k/N)、Notes、cache hit、表格横线 |
| 段头、表头、字段标签 | accent+bold `dark_cyan` (36) | `::` 段头、表格表头行、字段标签 |

> 表头行用 `accent_bold` 而非 `emphasis`：部分终端把"默认前景加粗"渲染成普通文本，表头会与数据行混淆；accent 在两套主题下都能读作表头（`emphasis` 保留给包名/实体名）。

**禁用**：`white`/`grey`（ANSI 97/37）与 `red`/`green`/`yellow`/`cyan`/`blue`（**高亮** 91/92/93/96/94）——明亮主题下几乎看不见；`dark_blue`（34）在深色主题下过暗。角色函数在颜色关闭时返回纯文本（crossterm 的 `SetAttribute` 不理会 `NO_COLOR`，会漏出 `\x1b[1m`）。

## 四、输出原语（8 个）
| 原语 | 内容 | 渲染 |
|---|---|---|
| section | 阶段头 | :: resolving… |
| plan | 事务摘要块 | Packages (3) … + 大小 |
| gate | 确认闸门 | :: Proceed with installation? [Y/n] |
| action(k,N,verb,pkg) | 计数动作行 | (1/3) downloading foo |
| row(id, Spin|Bar, msg) | 在途行（仅 TTY） | 转圈青 / 进度条蓝 |
| done/fail/notes | 包收尾 | ✓ foo / ✗ foo (err) / 注释 |
| info/warn/err | 旁注/前缀 | INFO/WARN /ERROR |
| summary | 结果汇总 | N installed, M upgraded, K failed |

## 五、并行逐包状态机 RowUI（泛化 BucketUpdateUI）
```
RowUI: start(id,label) / set_bar(id,cur,total) / set_spin(id,msg)
       / done(id) / fail(id,err) / finish_all()
```
- 下载条 + 转圈行共用同一个 MultiProgress，垂直对齐堆叠。
- 事件默认只发 start → done/fail 粗粒度；-v 注入细节；进度事件节流 + 仅 TTY。

## 六、确认闸门（三层可关）
| 层 | 开关 | 行为 |
|---|---|---|
| 强制开 | --confirm（新增） | 弹闸门，无视配置 |
| 强制关 | -y/--yes（已有） | 跳过 |
| 全局默认 | config.json confirm:false（新增） | 所有 sync 默认不弹 |
| 内置 | 无开关 | TTY 弹 / 非 TTY 不弹 |

计划块永远打印；关的只是「等 Y/n」。

## 七、verbosity 与颗粒度
| 级别 | 漂亮输出 | 日志 |
|---|---|---|
| 默认 | 最小 | ERROR |
| -v | 不变 | INFO |
| -vv | 不变 | DEBUG（排错） |
| -vvv | 不变 | TRACE |

RUST_LOG/HOK_LOG_LEVEL 模块级精细过滤。源头发射门控：低 verbosity 不构造/发送细粒度事件；进度事件节流；惰性格式化。

## 八、全命令输出
A. 同步事务类 install/update/upgrade/reinstall/uninstall/reset/cleanup：
```
:: resolving dependencies...
:: Building transaction...
Packages (4)  [安装] foo-1.0  [升级] bar-2.0 -> 2.1
             [移除] baz       [替换] old/new
Total download size: 3.4 MiB
:: Proceed with installation? [Y/n]          ← 可关
:: Installing...
⠋ resolving foo
(2/4) downloading foo [####----] 45%
(2/4) ✓ foo
:: Running post-install hooks...
WARN  baz: shim 冲突已覆盖
foo: 请配置 xxx
:: 2 installed, 1 upgraded, 1 failed
```
B. 查询类 list/search/info/status/which/cat/prefix/home/cache/bucket list/config list --all/depends/holds/missing-checkver/checkver/checkurls/checkhashes：:: 标题 + 对齐行 + 尾部计数；check* 复用 RowUI 逐包并行。
C. 仓库/配置 bucket/config/alias/export/import/cache/cleanup：bucket update 用 RowUI 转圈。
D. 杂项 create/formatjson/shim/virustotal/completions/ci-auto-pr：单结果行；virustotal 逐样本 (k/N)。

### 表格规则（所有「对齐行」共用；2026-09-12 固化）

| 规则 | 内容 |
|---|---|
| R1 | 缩进 2 空格 + 列距 2 空格 |
| R2 | 列宽 = max(表头, 全部单元格)，按**视觉宽度**（`format::visual_width`，CJK=2）；禁用 `{:<w$}`（Rust 宽度按字符数，中文会错位） |
| R3 | 表头 `style::emphasis` + `style::dim("-" * w)` 横线（= PowerShell 默认表格外观） |
| R4 | 空尾列不打印；空单元格不着色；纯数字列用 `format::pad_visual_right` 右对齐 |
| R5 | 标题 `output::header` 在**顶部**、统计 `output::info` 在**底部**——官方也是这么分的：`scoop-list.ps1:25`/`scoop-search.ps1:143,191` 的标题在顶，`scoop-cache.ps1:33`、`scoop-checkup.ps1:49-56` 的统计在底 |
| R6 | 渲染拆成 `layout(rows, headers) -> Vec<String>` + 打印包装，对齐/裁剪/右对齐可单测 |

已落地的表：`list`（名称/版本/源/状态）、`status`（+已安装版本/最新版本/缺少依赖项/信息）、`cache list`（名称/版本/大小，合计在底部）、`bucket list`（名称/清单数/更新时间/来源）、`bucket known`（名称/仓库）、`config list --all`（配置项/当前值/默认值/说明）。

`info` 不在其中：它是 label/value 块（官方 `scoop info` 同样没有表头/横线），只用 R1/R2/R4。

官方对照：`supporting/formats/ScoopTypes.Format.ps1xml` 一个文件声明 3 张表（ScoopApps / ScoopShims / ScoopStatus），**列集不同、机制共用**——统一的是规则层，不是形状。

## 九、回退矩阵
| 场景 | 行为 |
|---|---|
| TTY | 转圈/进度条/全彩色 |
| TTY + --no-progress | 关转圈/进度条，留彩色计数行 |
| TTY + NO_COLOR | 无彩色，留转圈/进度条 |
| 非 TTY | 无转圈/无进度条/无彩色，纯 (k/N) verb pkg |

## 十、实施阶段
1. ① 渲染层：彩色语义表 + 单渲染器 + RowUI（合并 MultiProgress）+ 计数行 + -v/-vv 单轴 + 砍 --detail/DETAIL_MODE/output::detail。（✅ 完成，b532f4d）
2. ② 确认：--confirm + confirm 配置 + 统一裁决函数。（✅ 完成，8076f49 / 4f58c0d）
3. ③ 并行：libscoop 逐包事件（统一包身份 + PackageInstallDone/Failed + 源头发射门控 + 进度节流）。
   - ③a 逐包计数与收尾（PackageCommitStart/Done/Failed）：✅ 完成，921b74b。
   - ③b 源头发射门控 + 进度节流：⏸ **评估后推迟**。进度节流已由 download 的 ProgressThrottle（100ms）+ 背压实现；细粒度（traced-only）事件发射开销经基准（crates/libscoop/benches/event_emission.rs，cargo bench -p libscoop）实测约 220–260 ns/事件，每包数十个事件合计仅微秒级，相对秒级安装可忽略；extract 进度是 stdout 展示、download 已节流，均不可/无需门控。详见 devlog `2026-08-29-phase3b-emit-gating-deferred.md`。
