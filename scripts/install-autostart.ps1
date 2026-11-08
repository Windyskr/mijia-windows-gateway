# 在当前用户的“启动”文件夹创建快捷方式并立即启动，不需要管理员权限。
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root "target\release\mijia-windows-gateway.exe"
if (-not (Test-Path (Join-Path $root ".env"))) { throw "找不到 .env，请复制 .env.example 后填写" }
if (-not (Test-Path $exe)) { throw "找不到 $exe，请先编译" }
$lnk = Join-Path ([Environment]::GetFolderPath("Startup")) "MijiaWindowsGateway.lnk"
$shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
$shortcut.TargetPath = $exe
$shortcut.WorkingDirectory = $root
$shortcut.Description = "米家温湿度计蓝牙广播采集与上传"
$shortcut.Save()
Get-Process -Name "mijia-windows-gateway" -ErrorAction SilentlyContinue | Stop-Process -Force
# 通过 Explorer 打开快捷方式启动，与登录时自启的方式相同，也不会随当前终端退出。
Start-Process -FilePath "explorer.exe" -ArgumentList ('"' + $lnk + '"')
Write-Host "已设置开机自启并启动：$lnk"
Write-Host "日志：$root\logs\agent.log"
