# hok status 对标分析（vs 官方 Scoop scoop status）

> 分析日期：2026-08；官方参照 D:\App\github项目\Scoop\Scoop\libexec\scoop-status.ps1（本地副本）、
> lib/core.ps1 的 app_status、supporting/formats/ScoopTypes.Format.ps1xml。hok 侧读
> src/cmd/status.rs 与 crates/libscoop/src/package/query.rs / install_state.rs / mod.rs。

---

## 一、官方语义（scoop-status.ps1）

官方 status 做两件事，输出风格偏告警清单：

### 1. Scoop 自身 / bucket 是否需要更新（网络检查，-l/--local 可关闭）

```powershell
# scoop-status.ps1:11-52
$no_remotes = $args[0] -eq '-l' -or $args[0] -eq '--local'   # 或 git 缺失
if (!$no_remotes) {
    $needs_update = Test-UpdateStatus $currentdir              # scoop 仓库 fetch origin
    foreach ($bucket in Get-LocalBucket) {
        if (Test-UpdateStatus (Find-BucketDirectory $bucket -Root)) { $bucket_needs_update=$true; break }
    }
}
if ($needs_update)      { warn "Scoop out of date. Run 'scoop update'..." }
elseif ($bucket_needs_update) { warn "Scoop bucket(s) out of date..." }
elseif (!$network_failure -and !$no_remotes) { success 'Scoop is up to date.' }
```
- -l/--local：只查本地已装 app，禁用 Scoop 自身与 bucket 的远程 fetch/检查；git 缺失自动等同 -l。
- Test-UpdateStatus：对 git 仓库执行 git fetch -q origin，再 git log HEAD..origin/<branch>，有提交即需更新；.git 不存在直接返回 true；fetch 失败（128）记 network_failure。

### 2. 已装 app 的状态清单（本地 + 全局两轮，scoop-status.ps1:54-77）

```powershell
$true,$false | ForEach-Object {          # global 与 local 各一轮
    $global=$_; Get-ChildItem (appsdir $global) | Where name -NE 'scoop' | ForEach-Object {
        $app=$_.name; $status = app_status $app $global
        # 只列出有问题的 app：outdated / failed / deprecated / removed / missing_deps
        if (!$status.outdated -and !$status.failed -and !$status.deprecated -and !$status.removed -and !$status.missing_deps) { return }
        $item.Name=$app; $item.'Installed Version'=$status.version
        $item.'Latest Version' = if ($status.outdated){$status.latest_version}else{""}
        $item.'Missing Dependencies' = $status.missing_deps -join ' | '
        if ($status.failed){$info+='Install failed'}  if ($status.hold){'Held package'}
        if ($status.deprecated){'Deprecated'}  if ($status.removed){'Manifest removed'}
        $item.Info=$info -join ', '; $list += [PSCustomObject]$item
    }
}
if ($list.Length -eq 0 -and !$needs_update -and !$bucket_needs_update -and !$network_failure) {
    success 'Everything is ok!'
}
```
- 枚举范围 = 已装 app（appsdir 目录），排除保留名 scoop，本地+全局都要看。
- 判定用 app_status（core.ps1:546-588）：version=Select-CurrentVersion；failed = app 目录存在但无有效 current/未装好；
  hold = install.json 的 hold；deprecated = bucket 的 deprecated/ 子目录里有同名 manifest；
  removed = 来源 bucket 已无该 manifest；latest_version=manifest.version；
  outdated：默认 Compare-Version (installed, latest) -gt 0；若 force_update 配置则 -ne 0（任何差异都算过期）。
  missing_deps = manifest.depends 中存在未装（!installed $app）的项。
- 输出：ScoopStatus 表格（Name / Installed Version / Latest Version / Missing Dependencies / Info）。

---

## 二、hok 现状（src/cmd/status.rs）

```rust
pub struct Args { #[arg(short='a', long)] all: bool }   // 官方是 -l/--local，hok 是 -a/--all

let packages = query::query(session, &["*"], &[QueryOption::Upgradable], false)?;  // installed=false !
for pkg in &packages {
    let upgradable = pkg.upgradable_version();
    if !all && upgradable.is_none() { up_to_date+=1; continue; }   // 默认只打过期
    line = "name/bucket version"; if let Some(new)=upgradable { push "-> new"; outdated+=1 }
    else { up_to_date+=1; }
    if pkg.is_held() { push " [held]" }
    output::status(&line);
}
if packages.is_empty() { t!("cmd.no_apps") }
else if all { if outdated==0 { ok } else { warn "Status: {outdated} outdated / {up_to_date} up to date." } }
else if outdated==0 { ok }
```
- 枚举范围错误：query(..., installed=false) -> query_synced（扫描全部 bucket 可用包目录），不是已装 app。实测 hok status -a 输出 21,286 行 ≈ 全部可用包目录；默认（非 -a）虽只打印 upgradable_version().is_some() 的项，但该函数对 synced 包依赖 installed_version() 二次比对，逻辑绕。
- 参数：hok 用 -a/--all（显示已最新 app），官方无此语义；官方 -l/--local（禁远程检查）hok 完全没有。

---

## 三、差距清单

| # | 维度 | 官方 Scoop | hok 现状 | 严重度 |
|---|------|-----------|---------|--------|
| 1 | 枚举范围 | 只枚举已装 app（local+global 两轮，排除 scoop） | query installed=false 扫全部可用 bucket 包（约 21k），非已装 | 高（-a 全量倾泻、与意图不符） |
| 2 | 已最新 app 是否列出 | 不列出（默认只列有问题的） | 默认不列；-a/--all 列所有 bucket 可用包（含未装）而非已装且最新 | 高 |
| 3 | Scoop/bucket 需更新提示 | 网络 fetch 后 warn Scoop/bucket out of date | hok 单二进制无 scoop 自更新；bucket 是否有新提交也未在 status 检查 | 设计差异；bucket 提示缺失 |
| 4 | -l/--local | 存在：禁远程 fetch，git 缺失自动降级 | 无此参数 | 中 |
| 5 | failed（装失败） | Info=Install failed | status 不检测（仅 list/checkup 路径有 broken 标记） | 中 |
| 6 | deprecated | bucket deprecated/ 子目录 -> Info=Deprecated | 全代码库无 deprecated 概念 | 中 |
| 7 | removed（来源 bucket manifest 被删） | Info=Manifest removed | status 未检测（query_installed 有 mark_bucket_removed，但 status 走 synced） | 中 |
| 8 | missing_deps | Info 列出缺失依赖名 | status 未计算 | 中 |
| 9 | hold | 作为 Info=Held package（与其它 info 并列） | [held] 后缀附加 | 风格差异 |
| 10 | force_update 配置 | outdated 判定变为任意差异 | status 用 upgradable_version()（仅版本更大） | 低 |
| 11 | Everything is ok! 判定 | 无列表 && 无 scoop/bucket 更新 && 无网络失败才打 | hok 按 outdated==0 打 ok（不看 bucket/scoop 状态） | 低 |
| 12 | 全局 app | $true,$false 两轮都查 | hok 走 session 当前 global 值，无双轮 | 中 |

---

## 四、根因与建议

根因：status.rs:19 query(session,&["*"],&[QueryOption::Upgradable], false) 的 installed=false 让它走了 query_synced（可用包目录扫描），而官方语义是枚举已装 app 并逐项 app_status。对比：list(list.rs:69) 与 update(update.rs:219/224) 都用 installed=true/query_installed，只有 status 用了 false——疑似与意图不符。

建议修复方向（若对齐）：
1. 改走 query_installed（installed=true）枚举已装 app，从而默认只报过期、-a 只含已装且最新；
2. 逐项补 app_status 同款判定：failed / deprecated / removed / missing_deps / hold / outdated；
3. 参数层面：若对齐官方应提供 -l/--local（禁远程检查）；现有 -a/--all 非官方语义，需决策保留还是改；
4. Everything is ok! 判定纳入无任何问题 +（可选）无 bucket 需更新；
5. deprecated / missing_deps 需先在 libscoop 层建模（当前全库无 deprecated）。

> 注：本文件为分析产物，不提交 git；结论可并入 .agents/skills/scoop-gap-audit.md 记忆。
