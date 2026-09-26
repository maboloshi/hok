# 表格输出统一：cache / bucket / config 表格化 + 统计位置规则

> 2026-09-12 ｜ 提交 `1fbcf42` `23b4401` `1c16d69` `a8d308f`

**背景（先调研、后动手，[[hok-investigate-first]]）**：`list`/`status` 之外的表各自手写且已经漂移——实测 `hok list` 行尾干净而 `hok status -a` 每行带一串尾随空格；`bucket known` 与 lib 层的 `config` 三列表用 `{:<w$}` 补宽（Rust 宽度按**字符数**，CJK 只算 1 列）会错位；`config list --all` 的分层违反（framework-review-2026-08 P1-5）仍在。调研给出 7 处表格清单 + 官方锚点 + 三方案对照后，用户拍板：**不做通用 Table 模板，只统一规则层**。

**官方锚点**（`D:\App\github项目\Scoop\Scoop`）：
- `supporting/formats/ScoopTypes.Format.ps1xml` 一个文件声明 3 张表（ScoopApps / ScoopShims / ScoopStatus），**列集不同、机制共用** → 统一的是规则，不是形状。
- 标题在**顶**：`scoop-list.ps1:25`、`scoop-search.ps1:143,191`；统计在**底**：`scoop-cache.ps1:33` `Total: N files, X MB`、`scoop-checkup.ps1:49-56` `Found N potential problems.`。
- `scoop cache show` = Name/Version/Length 三列（`scoop-cache.ps1:17-33`）；`scoop bucket known` 只输出桶名字符串（`lib/buckets.ps1:48-50`）。

**落地**：
1. `cache list`（方案 A）：官方三列（名称/版本/大小）、丢弃冗余的 `#hash.ext`、大小列右对齐、合计移到行下方独立成行、加 `show` 别名；`crates/libscoop/src/cache.rs::list` 补 `(name, version)` 排序——原来直接用 `read_dir` 顺序，只是 NTFS 恰好有序。
2. `bucket list`（变体 B2）：树形 → 4 列表格（名称 | 清单数 | 更新时间 | 来源，长 URL 放最后）+ 段头；`bucket known` 与 `list -k` 合并为同一渲染并加表头。
3. `config list --all`：lib 只返回数据（`ConfigRow` / `ConfigCell{Unset,Auto,Text,List}`），渲染上移 CLI；新增「说明」列（放最后，长路径不影响值列对齐）与段头；占位符本地化（`未设置`/`自动检测`）——旧实现把"未设置"和真值 `proxy = none` 都打成 `none`，CLI 无法区分；`private_hosts` 只列 match 模式，绝不打印带 token 的 `headers`；顺带补上 `hok config --help` 缺失的 4 项（jobs/editor/confirm/script-engine）。
4. `status` 行尾裁剪（空尾列不打印、空单元格不着色）；`list`/`status`/`cache`/`bucket`/`config` 五处补 `layout()` 单测（此前 `src/cmd` 只有 `info` 有测试）。
5. `format::pad_visual_right`（数字列右对齐）。

**坑/纪律**：
- 右对齐列的测试要断言**右边缘**；断言"值与列首同列"会失败（`2383` 在宽 6 的列里从第 3 列才开始）。
- 横线行不能用子串定位断言：`find("----")` 命中的是更长横线串的第一个位置，不是目标列。
- `format!` 的 `{:<w$}` 按字符数补宽，CJK 会错位——表格宽度一律走 `visual_width`。
- 表格行尾裁剪要按**列**裁剪（最后一列不补宽），不能对整个已着色行做 `trim_end`（ANSI reset 在末尾，尾随空格在它前面）。

**验证**：`cargo test --workspace`（除本沙箱 2 个 `permission::*` 环境失败）、`cargo test -p hok --test i18n_keys`、clippy（仅 innospect 既有 3+1 条）；用空格替换法手工核对了 `cache list`/`cache show`/`bucket list`/`bucket list -k`/`bucket known`/`config list --all`/`status -a`/`list` 的实际输出。

**遗留**：CJK 包名的缓存文件被 `REGEX_CACHE_FILE`（`[0-9a-zA-Z._-]+`）拒绝，`cache list`/`cache rm` 都看不到它——既有行为，未在本轮改动。→ **2026-09-25 已修**（`ff05b80`；根因是 name/version 字符类过窄，含 `+`/`_` 的版本号同样中招，见本日志 2026-09-25 条目）。

---
