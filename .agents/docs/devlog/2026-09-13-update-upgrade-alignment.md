# update/upgrade 对齐：删冷却、`-i`、hold 口径、引擎归位

> 2026-09-13 ｜ 分支 `fix/update-upgrade-align`，`ab7ea6e` → `77475e3`，6 commit

## 背景与决策

用户逐条拍板：①`--independent` 短选项对标官方改 **`-i`**（描述用官方 `Don't install dependencies automatically`）；②`--escape-hold` **删除**——"hold 原意仅锁更新，不应该有绕过"，据此**连 uninstall 的 hold 跳过一起删**，换被锁版本的正规路径是"卸载 → `hok install <app>@版本` → `hok hold`"；③`-f/--force` 与官方不一致（hok 兼做"绕过桶冷却"）；④`upgrade` 与 `update` 的关系选 **伞 + 引擎**（引擎搬进 `upgrade.rs`、`update` 反向引用），flag 只补 `-i`/`-k`；⑤发现 A（install 路径对 held 包**静默跳过**）修，但**不复用 `held_skip` 文案**（升级路径"保留原因" ≠ install 路径"你点名要装却被锁"）；⑥发现 B（`SyncOption::Cascade`/`resolve_cascade` 是死链）登记 **R5 单开会话恢复**。

调研结论（供后续参照）：官方显式 `scoop update` **无条件**同步自身+buckets（`libexec/scoop-update.ps1:397-398`），3h `LAST_UPDATE` 只门控**隐式自更新**（`lib/core.ps1:1168-1178`）；官方 **hold 唯一闸门**在 update 路径（`scoop-update.ps1:432-438`），install/uninstall 都不查；官方无 `upgrade` 命令（只是别名建议 `scoop-alias.ps1:14`）；pacman 无客户端时间窗（`-y` 按"库是否最新"、`-yy` 才强制，节流交给 systemd timer），hok 的 pacman 参照只在**输出风格**。

## 实现

`ab7ea6e` 引擎搬迁（纯 move）→ `c2581d4` 删冷却（`Config::update_cooldown_remaining[_at]` + 单测 + i18n `bucket_cooldown` + 两份 README + 失败汇总里的 `--force` 提示；`-f` 收敛为官方语义；`parse_last_update` 随即变 dead code ⇒ 保留 + `#[allow(dead_code)]` + 理由）→ `c25e39c` `-I`→`-i` → `20575a5` hold 口径 + 删 `-S`（CLI 四处 + `SyncArgs` 字段 + `SyncOption::EscapeHold` + 4 个 i18n 键；uninstall 不再跳过 held；install 路径改可见警告 `held_install_skip`；`resolve_cascade` 去形参、held 依赖恒报错）→ `ba6f923` upgrade 补 `-i`/`-k` → `77475e3` rustfmt。

## 验证

`cargo test --workspace` 仅 2 条既有 ACL 环境性失败（**已在未改动的主树复现同样失败**，属沙箱不允许改 ACL）；`clippy -p libscoop -p hok --all-targets --no-deps -- -D warnings` 干净；改动文件级 `rustfmt --edition 2024` 干净；`--test cli_help_i18n|i18n_keys|exit_codes` 全绿；进程级冒烟（自建 debug 二进制）：四条命令 help 面正确（`-i`/`-k` 在、`-S` 消失）、`hok update -S` 用法错 exit 1、`hok update -i <未装包>` 走到引擎并 exit 0、zh 帮助新键有中文。落地用 `worktree-merge.ps1 -VerifiedRef 77475e3`（无需 rebase，漂移校验逐字节相同）。

## 教训

1. **删配置项要顺着查读者**：删 `update_cooldown_remaining` 后 `internal/time.rs::parse_last_update` 立刻变 dead code —— `-D warnings` 会抓；本仓惯例是**保留 + `#[allow(dead_code)]` + 写明理由**（`constant.rs:72-78`），因为 `last_update` 仍在写（官方 config 面）且 `status --local` 新鲜度检查是待做项。
2. **删 CLI flag 远不止删字段**：帮助文档注释 + 两个 locale 的 `cli_help.*` 键 + README 的"特性宣称" + **报错文案里的提示**都要清（差点漏掉 `bucket_update_failed_summary` 里的 "run 'hok update --force' to retry"）。
3. **"去重参数"要先过量化门槛**：`hok-code-organization` 有 2026-09-10 的否决案例（同类 flatten 净 +54 行），所以本轮**只补字段、不做 macro/flatten**——重复是既有风格，"可漂移的重复"才值得动。
4. **`replace_all` 之后要复查**：`resolve_cascade(&session, &mut packages, false)` 的三处调用里有一处没被替换掉，靠 `cargo check --workspace --all-targets`（要 `--all-targets` 才编测试）才暴露；改签名类改动必须跑 `--all-targets`。
5. **官方没能力 ≠ 可以删**：`Cascade` 链已测通但生产不可达（`-c/--cascade` 在 `5f2532e` 被误删），用户裁决是**单开会话恢复**而不是清掉——遇死代码先问"是没接线还是不要了"。

---
