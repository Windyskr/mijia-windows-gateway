# 取消开机自启并停止采集程序；还没上传的读数保留在 data\outbox.db，下次启动后继续上传。
$lnk = Join-Path ([Environment]::GetFolderPath("Startup")) "MijiaWindowsGateway.lnk"
Remove-Item $lnk -ErrorAction SilentlyContinue
Get-Process -Name "mijia-windows-gateway" -ErrorAction SilentlyContinue | Stop-Process -Force
Write-Host "已取消开机自启并停止采集程序"
