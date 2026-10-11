# 待办复核落地：bucket 优先级、安装态回落可见性、Start Menu 解析

> 2026-10-11 ｜ 提交 `28b04f3`, `e8c999a`, `ed308e5`, `4df879c` ｜ worktree `.worktrees/pending-review` ｜ 分支 `fix/pending-review`

一次「复核 → 评估 → 落地」批次：按 `investigate-first` 先给 B1/X6/C2/MS1/N2/O3B/C1 七条的评估报告（证据 + 选项 + 推荐），
用户拍板「先按推荐走」，随后落地四条、三条转长期登记。

## 落地

- **B1（`28b04f3` + `e8c999a`）**：上游确有 bucket 优先级——`Get-LocalBucket` 把 known 桶按 `buckets.json`
  序提前，`Find-AppBucket` 取第一个。先修生成端：build-dependency 的 `serde_json` 缺 `preserve_order`，
  使 `KNOWN_BUCKETS`/`BUCKET_PRIORITY_ORDER` 实为**字母序**（`main` 落在第 4 位，生成的
  `out/known_buckets.rs` 可证），`bucket known` 输出的也是这个错序；守卫测试原先把两个生成结果互比（恒真），
  现改为钉字面官方序。再把优先级接进 `select_candidate`（`extras` 曾压过 `main`），排名收敛为
  `bucket::known_rank`——`bucket list` 本来就自带同款查找，已并入，`BUCKET_PRIORITY` 的 `dead_code` allow 随之删除。
- **MS1 点 1（`ed308e5`）**：junction 模式下 `current` 清单不可解析时 `resolve_current_version` 回落到版本目录，
  而那个版本正是 `cleanup` 保留的 ⇒ 判错就保错目录。回落本身与官方一致（`Get-InstalledVersion`），
  差异只在**可见性**：官方由 `parse_json` 发默认可见告警，hok 的同类 tracing 告警要到 `-v`/`-vv`。
  `cleanup` 现补一条默认可见告警并点名将被保留的版本，解析与控制流不变。
- **N2（`4df879c`）**：`shortcut_dir` 的用户范围原走 `config_dir()` 链，缺 `%APPDATA%` 时会落到
  `%USERPROFILE%\.config\...`（不是开始菜单）。改读注册表 `Shell Folders`，对齐官方
  `GetFolderPath('StartMenu'|'CommonStartMenu')`；环境变量链保留为兜底。

## 转长期登记

- **X6** 清单 hash 前缀大小写：hok 与官方 `schema.json` 同严，是「严于官方运行时」而非漏判。
- **C2** nightly 判定大小写：对齐方向是把「跳过 hash 校验」扩到大小写变体上，属放宽而非修正。
- **X7**（原 C1 的 7z 半边）`use_external_7zip`：上游原意是依赖归属（不装 helper、改用 PATH 上的 `7z.exe`），
  hok 主力解压链不走 `7z.exe`，要做必须与 X2 同批；C1 因此只剩 `show_manifest`。
- **O3B** 维持暂缓：结论与重启条件早已落在 `output-system-design.md` §10 与 `2026-08-29` 基准。

## 复核纠正的两处条目前提

- 本机 4 桶 22,845 份清单（大小写敏感匹配）：大写 hash 前缀 **0 例**、`"version": "Nightly"` **0 例** ⇒
  两条都是「零真实用例」，这是它们转登记而非落地的主因。
- MS1 的「静默」只部分成立：`parse_file`/`parse_install_info` 一直有 tracing warn，`list` 也早把坏安装显示为
  「安装失败」，真正缺的是决策后果的默认可见性。N2 的 `psmodule.rs` 半边**不成立**（兜底为空串，
  不产生相对路径，且比官方更安全），已从条目移出。

## 证据

- 新增测试：`bucket_priority_matches_official_order`、`known_rank_orders_official_buckets_first`、
  `resolve_multiple_candidates_prefers_official_bucket`、`cleanup_warns_when_current_manifest_is_unreadable`、
  `shortcut_dir_composes_the_start_menu_path`。
- 回归探针：把 `known_rank` 临时改成恒 `usize::MAX`（等价旧字母序），新测试失败并给出 `left: ["aaa","main"]`。
- 门禁：`clippy --workspace --all-targets` 0、`cargo test --workspace` 0 failed、改动文件 `rustfmt --check` 干净。
