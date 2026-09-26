# S5：`checkup` 对齐官方环境检查 + 新增 `health`

> 2026-09-13 ｜ 提交 `5ae994d`

**背景**：审计报告把 `checkup` 列为"基线同名异义"——官方 `scoop checkup` 是环境诊断，hok 是已装包结构体检。按 investigate-first 出报告（`.agents/docs/reports/s5-checkup-investigation-2026-09.md`），用户拍板 **B&C 并存**、`checkup` 恒 exit 0、Defender 纯 Rust。

**做法**：`checkup` 重写为官方 9 项环境检查（管理员+Defender 性能×2 / main bucket / 长路径 / 开发者模式 / 7zip·innounp·dark / 双根 NTFS），顺序与文案对齐 `libexec/scoop-checkup.ps1`；已装包体检迁到新命令 `health`（`package::checkup`→`package::health`）。判定逻辑做成纯 `evaluate(&EnvFacts)`（10 条离线单测），OS 采集与探针分在 `diagnose` 与 `internal::os`（新增 winreg 读注册表、advapi32 查服务、kernel32 查卷格式）。`hok checkup` 的 `!apps_dir.exists()` 不可达分支随迁移修掉。

**坑 / 决策**：
- **Defender**：不 shell 出 `Get-MpPreference`（省 ~0.5–1s pwsh 冷启动），改读 `HKLM\SOFTWARE\Microsoft\Windows Defender`（实时开关 + `Exclusions\Paths` 子键）+ `WinDefend` 服务状态；**任一读不到即整体降级为"不报"**（对齐官方"没有 Get-MpPreference 就跳过"）。Defender 仅在管理员下检查，`WDAGUtilityAccount` 跳过。
- **`use_external_7zip`** hok 未建模 ⇒ 7zip 恒按需检查（官方仅在未设该键时检查）。
- **e2e 隔离**：`hok` 的旁置 `config.json` 只是 **Scoop 兜底源**，hok 自身的 `%USERPROFILE%\.config\hok\config.json` 会覆盖它——隔离要改 `USERPROFILE`，不是改旁置文件。
- 输出流：`checkup` 的问题走 stderr、建议走 stdout（沿用 hok 约定）；管道里两流合并顺序不保证，属预期。

**验证**：`cargo test --workspace` 仅 `permission::tests::grant_users_write_inherit_*`（沙箱 ACL error 5，clean HEAD 同失败）；隔离 `USERPROFILE` 实测 `checkup` 6 问题 exit 0、`health` 命中"无 apps 目录"分支 exit 0；clippy 新增 0、rustfmt 干净。
