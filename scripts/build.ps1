# 没有 Visual Studio 生成工具（MSVC）时的编译方式：用 llvm-mingw 编译 x86_64-pc-windows-gnullvm 目标，
# 静态链接 C 运行库，生成的 exe 不依赖额外 DLL。有 MSVC 的话直接 cargo build --release 即可。
# 用法：powershell -ExecutionPolicy Bypass -File scripts\build.ps1 -LlvmMingw <llvm-mingw 解压目录>
param([Parameter(Mandatory = $true)][string]$LlvmMingw)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$hostToolchain = "stable-x86_64-pc-windows-gnu"
$target = "x86_64-pc-windows-gnullvm"
$sysroot = & rustc "+$hostToolchain" --print sysroot
if ($LASTEXITCODE -ne 0) { throw "请先执行 rustup toolchain install $hostToolchain --profile minimal" }
$clang = Join-Path $LlvmMingw "bin\x86_64-w64-mingw32-clang.exe"
if (-not (Test-Path $clang)) { throw "找不到 $clang，请让 -LlvmMingw 指向 llvm-mingw 的解压目录" }
$names = @("PATH", "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER", "CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER", "CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_RUSTFLAGS")
$before = @{}
foreach ($name in $names) { $before[$name] = [Environment]::GetEnvironmentVariable($name, "Process") }
try {
    $env:PATH = (Join-Path $LlvmMingw "bin") + ";" + $env:PATH
    # 构建脚本按宿主（windows-gnu）编译，用工具链自带的 gcc 链接；正式产物用 llvm-mingw 的 clang 链接。
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = Join-Path $sysroot "lib\rustlib\x86_64-pc-windows-gnu\bin\self-contained\x86_64-w64-mingw32-gcc.exe"
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_LINKER = $clang
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNULLVM_RUSTFLAGS = "-C target-feature=+crt-static"
    & cargo "+$hostToolchain" build --release --locked --target $target --manifest-path (Join-Path $root "Cargo.toml")
    if ($LASTEXITCODE -ne 0) { throw "编译失败，请确认已执行 rustup target add $target --toolchain $hostToolchain" }
    $output = Join-Path $root "target\release"
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    Copy-Item (Join-Path $root "target\$target\release\mijia-windows-gateway.exe") $output -Force
    Write-Host "编译完成：$output\mijia-windows-gateway.exe"
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $before[$name], "Process") }
}
