# PS 兼容层 `-Switches`/`-Overwrite` 生效，参数声明对齐官方

> 2026-10-11 ｜ 提交 `caa1ba6`, `56d3083` ｜ worktree `.worktrees/x5-ps-decompress-switches`, `.worktrees/x5b-param-shape` ｜ 分支 `fix/ps-decompress-switches`, `fix/ps-decompress-param-shape`

计划项 X5（X3 修复后才真正可达的两条偏离）。`Expand-*Archive` 只写
`[[HOK_EXTRACT]]` 标记、由 Rust 在整段脚本跑完后统一解压，所以参数必须在**协议**里活下来。

## 路线：marker 加字段 + 交给外部 CLI（不是"在 Rust 里翻译开关"）

- 官方 `lib/decompress.ps1:99-101` 把 `-Switches` 原样 `-split` 进 7z 命令行；`:77-79,102-106`
  把 `-Overwrite All|Skip|Rename` 映射成 `-aoa/-aos/-aou`；`:199-201,244-254` 同样把 `-Switches`
  交给 lessmsi / innounp。这些开关**只对那个 CLI 有语义**（本机语料：`-p*` 29、`-x!`/`-xr!` 22、`-t*` 24…），
  在 Rust 里复刻 = 重实现 7z 的加密/通配符/类型嗅探 ⇒ 采纳"带字段的 marker 改走外部 CLI"。
- marker 扩为七段（`format|source|dest|extract_dir|removal|switches|overwrite`）；**只有带这两个字段**的
  marker 走 CLI，其余走原样原生路径 ⇒ 无参数调用行为不变。依赖早已具备（脚本含 `Expand-7zipArchive`
  → 注入 `7zip`，含 `Expand-InnoArchive` → `innounp`），且 `Expand-DarkArchive` 本来就在 PS 层直调外部工具。

## 收尾：参数声明跟官方到底

PowerShell 规则：**只要函数里有一个参数显式声明 `Position`，其余未声明者就只能按名绑定**。
官方只给 `Path`/`DestinationPath` 显式 `Position`（`:66-71`），于是第三个位置参数落入 `$Switches`。
hok 原来让 `ExtractDir` 隐式 `Position = 2`，把 `wpsoffice-cn-x64.json:23-24` 的 `'-bsp1'` 当成子树名
⇒ **静默解出空目录**。现四个 helper 均按官方声明形状（`ExtractDir`/`Switches`/`Overwrite` 只按名绑定），
`-Overwrite` 同样保持 named-only（散落的位置参数必须抵达 `$Switches`，而不是撞 ValidateSet）。

## 官方对照要点

`-Switches` 原样转交（`:99-101`）、`-Overwrite` → `-ao*`（`:102-106`）、7z 参数序（`:94-106`）、
`-ExtractDir` → `-ir!` + movedir 上提 + 删空顶层（`:96-98,122-129`）、tar 二次解包时递归调用**丢掉**
`$Switches`（`:95,111-121`，hok 同）。**有意不跟**：`:93` 的 `$DestinationPath.TrimEnd('\')`——
hok 也把该值交给 `-o`，但 `C:\` 经 TrimEnd 会变盘符相对路径 `C:`，不跟更正确（维持原登记）。

## 证据与门禁

- `-Switches` 真生效：真 7z.exe 造 `-mhe=on -pHOK-PW` 夹具（对位 `-pmuggle` 一族）→ 原生取不到内容，
  marker 带 `-pHOK-PW` 后内容正确；`-xr!*.pdb` 端到端确认被排除。
- `-Overwrite` 三态（All/Skip/Rename）行为与官方一致；全链路 `-Overwrite 'Skip'` 保留旧文件。
- **零回归**：语料对拍 1904 条可求值调用，`identical=665 / both-err=1216 / different=23`，
  23 条全部含 `-Overwrite`（原被吞的那批）。**未做**全量分词归因（纯文本无法区分
  `-ExtractDir 'x'` 与 `$dir '-bsp1'`），已按此如实登记；`wpsoffice-cn-x64` 那两行是已知实例。
- **真 MSI 端到端**（X5b 补）：`crates/libscoop/tests/fixtures/msi/x5b-fixture.msi` +
  `gen-fixture.ps1`（WindowsInstaller COM），证明 `Expand-MsiArchive -Switches` 抵达 lessmsi 命令行。
- 变异测试防假绿：删掉 `-Overwrite` 参数 → 参数探针 `MISSINGPARAM`/`ValidateSet` 报警 + 全链路用例 FAILED。
- 门禁：`clippy -p libscoop --all-targets` 0、`test -p libscoop` 0、两文件 `rustfmt --check` 0；
  X5b 重放后在含前四批的树上再跑 `clippy --workspace` 0、`test --workspace` 0。
