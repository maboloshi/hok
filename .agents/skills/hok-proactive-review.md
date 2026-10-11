---
name: hok-proactive-review
title: hok 提交前 review 的项目专属对照项
description: hok 提交前 review 时读（通用门见全局 proactive-review）：官方 Scoop 行为锚点核对、五层架构合规、去重门槛、命名对齐，以及曾因"提交后才 review"产生补丁提交的历史教训。
metadata:
  type: project
---

**通用门（提交前做、七项清单、输出要求）见全局 skill `proactive-review`**；本文件只补 hok 专属部分，不重复通用条款。

## hok 专属对照项
1. **官方行为锚点**：读本地官方 Scoop 源码（路径见 [[local-scoop-source-path]]）核对语义——参数含义、错误处理（error+continue vs abort）、幂等性、守卫条件、schema 字段归属；结论带 file:line。
2. **架构合规**：五层单向依赖、模块职责单一（命名即文档）、剥离彻底性（无泄漏的依赖语义，如 permission 不依赖 persist_root）——细则见 [[hok-code-organization]]。
3. **去重门槛**：**本体见全局 skill `proactive-review` §7**（本文件不复述）；本仓判例（P2-6 clap `flatten`）见 [[hok-code-organization]]。
4. **命名对齐官方**（如 `persist_permission` 而非 `ensure_global_permission`）。

## 本仓易漏项（每条都是提交前要过的检查）
- **失败中断 vs 继续**：官方 `reset` 用 `ForEach` error+continue，hok 曾失败即整体中断 ⇒ 对照官方的错误处理再提交。
- **ACL 幂等性**：`SetEntriesInAclW` 的 GRANT 合并不得累积 ACE ⇒ 必须有读回 DACL 的断言。
- `TRUSTEE_IS_SID=0` 是 SDK 正确值（勿按记忆改成 3）。
- 单 app reset 的 global 自动检测缺失（既有差异，记录未修）。
- **P2-6 clap 参数组 flatten**：去重门槛的典型反面案例（净增代码 + 引入第三种写法）→ **提交前跑一次 `git diff --numstat` 就能否决**；案例数字见 [[hok-code-organization]]。
