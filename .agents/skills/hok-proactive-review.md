---
name: hok-proactive-review
title: hok 提交前 review 的项目专属对照项
description: hok 专属：提交前 review 的通用门与清单见全局 proactive-review；此处只记本仓对照项（官方 Scoop 源码、五层架构合规、去重门槛、命名对齐）与历史教训。
metadata:
  type: project
---

**通用门（提交前做、七项清单、输出要求）见全局 skill `proactive-review`**；本文件只补 hok 专属部分，不重复通用条款。

## hok 专属对照项
1. **官方行为锚点**：读本地官方 Scoop 源码（`../Scoop\libexec|lib`，见 [[local-scoop-source-path]]）核对语义——参数含义、错误处理（error+continue vs abort）、幂等性、守卫条件、schema 字段归属；结论带 file:line。
2. **架构合规**：五层单向依赖、模块职责单一（命名即文档）、剥离彻底性（无泄漏的依赖语义，如 permission 不依赖 persist_root）——细则见 [[hok-code-organization]]。
3. **去重门槛**（2026-09-10 补）：去重类改动必须给出 ①净行数（`git diff --numstat`）②新增间接层 ③是否引入新写法 ④覆盖范围；**净增代码/净增间接层且真实问题无证据 → 否决**。
4. **命名对齐官方**（如 `persist_permission` 而非 `ensure_global_permission`）。

## 历史教训（这些曾因「提交后才 review」产生补丁 commit，提交前就应抓住）
- `reset -a` 失败即整体中断（官方 ForEach error+continue）→ 事后补 fix `14541c7`。
- `SetEntriesInAclW` 幂等性（GRANT 合并不累积 ACE）→ 事后补测试 `458aa8c`。
- `TRUSTEE_IS_SID=0` 是 SDK 正确值（勿按记忆改成 3）。
- 单 app reset 的 global 自动检测缺失（既有差异，记录未修）。
- **P2-6 clap 参数组 flatten**（2026-09-10）：净 +54 行 + 14 处 `.enabled()` 间接层，只覆盖 3 个从未漂移的 flag，且引入与既有「结构体 + 重复字段 + trait 投影」并存的第三种写法 → 用户追问「是否为了合并而合并」后回退。**提交前跑一次 `git diff --numstat` 就能否决。**
