[CmdletBinding()]
param(
    [string]$InnoCompiler
)

$ErrorActionPreference = 'Stop'
$project = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '../..')).Path
$target = 'x86_64-pc-windows-msvc'
$manifest = Join-Path $project 'Cargo.toml'
$buildDirectory = Join-Path $project 'target'
$output = Join-Path $buildDirectory 'installer'
$previousRustFlags = $env:RUSTFLAGS
$cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
if (!$cargo) {
    $cargo = Get-Command (Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe')
}
if (!$InnoCompiler) {
    $InnoCompiler = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
}
if (!$InnoCompiler) {
    $InnoCompiler = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
}
if (!$InnoCompiler) { throw 'Install Inno Setup or pass the full path to ISCC.exe.' }
$compiler = Get-Command $InnoCompiler

Push-Location -LiteralPath $project
try {
    $env:RUSTFLAGS = "$previousRustFlags -C target-feature=+crt-static".Trim()
    $metadata = & $cargo.Source metadata --locked --no-deps --format-version 1 --manifest-path $manifest
    if ($LASTEXITCODE -ne 0) { throw 'Could not read the application version.' }
    $version = ($metadata | ConvertFrom-Json).packages[0].version

    & $cargo.Source build --locked --release --target $target --target-dir $buildDirectory
    if ($LASTEXITCODE -ne 0) { throw 'The release build failed.' }

    $app = Join-Path $buildDirectory "$target/release/elearnin.exe"
    $script = Join-Path $PSScriptRoot 'Elearnin.iss'
    & $compiler.Source "/DAppVersion=$version" "/DAppBinary=$app" "/O$output" $script
    if ($LASTEXITCODE -ne 0) { throw 'The installer build failed.' }

    $name = "Elearnin-$version-windows-x64-setup.exe"
    $installer = Join-Path $output $name
    $hash = (Get-FileHash -LiteralPath $installer -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$installer.sha256" -Value "$hash *$name" -Encoding ascii
    Get-Item -LiteralPath $installer, "$installer.sha256"
} finally {
    $env:RUSTFLAGS = $previousRustFlags
    Pop-Location
}
