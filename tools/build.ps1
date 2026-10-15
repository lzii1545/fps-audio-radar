# 用 rustup 的 gnullvm 工具链构建
# 链接依赖（见 .cargo/config.toml）：
#   - rust-lld（工具链自带）
#   - windows-gnu 工具链的 self-contained 导入库（需 rust-mingw 组件）
#   - tools\mingw-libs（DirectX/shlwapi 别名库）
param([string[]]$CargoArgs)
if (-not $CargoArgs -or $CargoArgs.Count -eq 0) { $CargoArgs = @('build') }

$tc = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnullvm"
$cargo = "$tc\bin\cargo.exe"
if (-not (Test-Path $cargo)) {
    Write-Error "gnullvm toolchain missing, run: rustup toolchain install stable-x86_64-pc-windows-gnullvm"
    exit 1
}
$gnu_lib = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\lib\self-contained"
if (-not (Test-Path $gnu_lib)) {
    Write-Error "mingw import libs missing, run: rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal --component rust-mingw"
    exit 1
}

# rust-lld 加入 PATH（.cargo/config.toml 里 -Clinker=rust-lld 依赖它）
$env:PATH = "$tc\lib\rustlib\x86_64-pc-windows-gnullvm\bin;$env:PATH"
& $cargo @CargoArgs
exit $LASTEXITCODE
