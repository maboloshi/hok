# unsafe 审计清单 — 阶段 5（unsafe_code = deny 落地）

> 生成：2026-08 · 分支 refactor/edition-2024 · 配合 .agents/docs/reports/refactor/prompts/phase-5-prompt.md
> 目的：逐一审计仓库内所有 unsafe 的用途，分类为「可消除 / 必要 FFI / 误报」，
> 作为 unsafe_code = deny 落地的前提与豁免清单。

## 结论摘要

| Crate | deny? | unsafe 数 | 处理 |
|---|---|---|---|
| hok（root） | ✅ deny | 0 | 无 unsafe，直接 deny |
| libscoop | ✅ deny | 18→16 | env.rs 1 处已消除；os.rs/permission.rs 模块级豁免 |
| hok-i18n-derive | ✅ deny | 0 | proc-macro，零 unsafe，直接 deny |
| hok-shim | ⚠️ deny + crate 级 allow | 53 | no_std FFI 本质，整 crate 豁免（方案 B） |
| innospect | ✅ deny（既有） | 0 真实 | 1 处「unsafe」字样仅出现在注释中，误报。**2026-09-19 起 vendored crate 已退役**，改用 registry `innospect 0.1.4`（其 manifest 自带同一组 deny lints） |

## 逐处审计

### crates/hok-shim/src/lib.rs（47 处）— 必要 FFI（shim 本质）
no_std、no alloc、固定缓冲区的 Scoop shim 生成器，整个实现就是 Win32 FFI：
- shim_memcpy / shim_memset / shim_memcmp（#![no_std] 下为编译器生成的调用提供 CRT 语义）
- unsafe extern "system" { … } Win32 声明块（CreateProcessW、GetModuleFileNameW、Job 对象等）
- core::slice::from_raw_parts 固定缓冲区读写、u16_strlen、expand_env、create_job、do_elevate、entry 等
- **分类**：必要 FFI，deny 不现实 → **crate 级 #![allow(unsafe_code)]（方案 B，附理由注释）**

### crates/hok-shim/src/bin/hok-shim.rs / hok-shim-gui.rs（各 3 处）— 必要 FFI
panic handler 调 hok_shim::write_stderr、entry point 调 hok_shim::entry()。
- **分类**：必要 FFI → 随 crate 一并 #![allow(unsafe_code)]

### crates/libscoop/src/internal/os.rs（15 处）— 必要 FFI（Win32 进程/提权）
- 4 个 unsafe extern "system" 块（kernel32 进程枚举、shell32 IsUserAnAdmin、ShellExecuteExW、ShellExecuteW）
- CreateToolhelp32Snapshot / Process32FirstW / OpenProcess / QueryFullProcessImageNameW / CloseHandle 等进程扫描
- **分类**：必要 FFI → **模块级 #![allow(unsafe_code)]**

### crates/libscoop/src/permission.rs（3 处）— 必要 FFI（Windows ACL）
GetNamedSecurityInfoW / SetEntriesInAclW / SetNamedSecurityInfoW（windows-sys 原生 ACL）。
- **分类**：必要 FFI → **模块级 #![allow(unsafe_code)]**

### crates/libscoop/src/internal/env.rs（1 处）— 可消除 ✅ 已消除
to_utf16_bytes 用 std::slice::from_raw_parts 把 Vec<u16> 重解释为 Vec<u8>。
- **分类**：可消除（非 FFI，纯字节重解释）→ 用安全 u16::to_le_bytes() 循环替代
- **提交**：950489a refactor(libscoop): drop unsafe byte reinterpretation in env.rs

### crates/innospect/src/lib.rs（1 处「unsafe」字样）— 误报
unsafe 仅出现在第 36 行 // The … unsafe_code, … 注释中，非真实 unsafe 代码。
- **分类**：误报（注释）→ 无需处理；[lints.rust] unsafe_code = "deny" 已生效

## 豁免清单（局部 allow）
- libscoop：internal::os、permission（模块级 #![allow(unsafe_code)]）
- hok-shim：crate 级 #![allow(unsafe_code)]（lib.rs + 2 个 bin）
- 其余 crate（hok / libscoop 其余模块 / hok-i18n-derive / innospect）零 allow，deny 全量生效

## 落地提交
- 950489a refactor(libscoop): drop unsafe byte reinterpretation in env.rs
- 10d7cb4 chore: enable unsafe_code = deny guardrail across safe crates