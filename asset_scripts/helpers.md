# hok 嵌入式 PS helper 函数

编译进 `hok.exe` 的 PowerShell 辅助函数，在执行包脚本时注入。
覆盖分析：19981 manifests from main/extras/scoop-private/scoop-app buckets。

## core.ps1（19 个）

| 函数 | 用途 | 使用次数 |
|------|------|---------|
| `Get-HelperPath` | 查找辅助工具路径（7z、innounp 等） | 2 |
| `friendly_path` | 路径缩写（Scoop 路径→ `~\..\` 格式） | 20 |
| `ensure` | 确保目录存在（如不存在则创建） | 356 |
| `is_admin` | 检测当前进程是否有管理员权限 | 159 |
| `New-DirectoryJunction` | 创建目录符号链接（用于重定向 AppData） | 23 |
| `info` | 绿色文字信息输出 | 342 |
| `warn` | 黄色文字警告输出 | 80 |
| `error` | 红色文字错误输出 | 839 |
| `abort` | 红色错误输出 + 抛出异常终止脚本 | 23 |
| `Invoke-ExternalCommand` | 调用外部命令（支持 `-RunAs` 提权、`-Quiet` 静默） | 101 |
| `WriteReg` | 写注册表（`Set-ItemProperty`） | — |
| `Get-RegKey` | 读取注册表值 | — |
| `New-RegKey` | 创建注册表项 | — |
| `Remove-RegKey` | 递归删除注册表项 | — |
| `Write-Env` | 设置环境变量（支持 `-Global` 参数选 Machine/User 范围） | — |
| `Remove-Env` | 删除环境变量 | — |
| `Get-InstalledVersion` | 获取 `apps/{app}/` 下最新版本目录名 | — |
| `Select-CurrentVersion` | 读取 `current` 符号链接指向的版本 | 3 |
| `Get-Version` | 同 `Get-InstalledVersion` | — |

## decompress.ps1（7 个）

| 函数 | 用途 | 使用次数 |
|------|------|---------|
| `Expand-7zipArchive` | 用 7z 解压（支持 `-ExtractDir`、`-Removal`、`-Switches`） | 386 |
| `Expand-InnoArchive` | 用 innounp 解压 Inno Setup 安装包 | 21 |
| `Expand-MsiArchive` | 用 msiexec 解压 MSI | 46 |
| `Expand-7ZipArchive` | **别名**，同 `Expand-7zipArchive` | — |
| `Expand-Msi` | **别名**，同 `Expand-MsiArchive` | 46 |
| `Expand-ZipArchive` | **别名**，同 `Expand-7zipArchive` | 5 |
| `Expand-DarkArchive` | **别名**，同 `Expand-7zipArchive` | 5 |

## 原生 PS cmdlet（2 个，不嵌入）

| 命令 | 用途 | 使用次数 |
|------|------|---------|
| `Stop-Service` | 停止 Windows 服务 | 34 |
| `Start-Service` | 启动 Windows 服务 | 3 |

> `Stop-Service`/`Start-Service` 是 PowerShell 原生 cmdlet，无需嵌入就会自动可用。

## 注

使用次数来自 `main/extras/scoop-private/scoop-app` 四个 bucket 的统计。
"—" 表示不在上述 bucket 的 profile 清单中使用，但因 Scoop 兼容性保留。
