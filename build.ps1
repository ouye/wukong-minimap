#requires -Version 5.1
<#
.SYNOPSIS
    Builds b1sdk.lib (MSVC) and then wukong_minimap.dll (Rust).

.EXAMPLE
    .\build.ps1
    .\build.ps1 -Full                     # compile every SDK translation unit
    .\build.ps1 -Install "D:\Games\Steam\steamapps\common\BlackMythWukong\b1\Binaries\Win64"
#>
param(
    [switch]$Full,              # -DB1SDK_FULL=ON
    [switch]$Asserts,           # -DB1SDK_ASSERTS=ON
    [switch]$Clean,
    [switch]$NoRustupDefault,   # don't touch the rustup default toolchain
    [string]$Generator,         # force a CMake generator, e.g. "Visual Studio 17 2022"
    [string]$Install,           # game Win64 folder; copies the built dll there
    [switch]$Package            # also zip a distributable into dist\
)

$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot

function Step($m) { Write-Host "`n==> $m" -ForegroundColor Cyan }
function Info($m) { Write-Host "    $m" }

# ---------------------------------------------------------------- cmake ---
Step "checking toolchain"
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
    throw "cmake not found on PATH. Install it: winget install Kitware.CMake"
}
Info "cmake : $(& cmake --version | Select-Object -First 1)"

# ------------------------------------------------------------------ msvc ---
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path $vswhere)) {
    throw "vswhere.exe not found -> no Visual Studio / Build Tools installed. Install 'Desktop development with C++'."
}

# every install that actually has the x64 C++ toolset
$installs = @()
& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
           -format value -property installationPath 2>$null | ForEach-Object {
    if ($_) {
        $p = $_
        $line = & $vswhere -path $p -format value -property catalog_productLineVersion 2>$null
        $installs += [pscustomobject]@{ Path = $p; Line = "$line" }
    }
}
if ($installs.Count -eq 0) {
    throw "Visual Studio is installed but the x64 C++ toolset (VC.Tools.x86.x64) is missing. Add 'Desktop development with C++' in the VS Installer."
}
Info "MSVC installs with the C++ toolset:"
$installs | ForEach-Object { Info "  [$($_.Line)] $($_.Path)" }

# Map a VS product line to its CMake generator name.
# vswhere's catalog_productLineVersion is "2019"/"2022" for older releases but
# reports the 2026 release as "18" (the major version), so map both spellings.
$genFor = [ordered]@{
    '2019' = 'Visual Studio 16 2019'
    '2022' = 'Visual Studio 17 2022'
    '2026' = 'Visual Studio 18 2026'
    '18'   = 'Visual Studio 18 2026'
}

if (-not $Generator) {
    # Only offer generators this CMake actually knows about, and prefer the
    # boring one: VS 2022 support is universal, VS 2026 (v18) needs a very
    # recent CMake.
    $cmakeHelp = (& cmake --help) -join "`n"
    foreach ($line in @('2022', '2019', '2026', '18')) {
        $cand = $genFor[$line]
        if (($installs | Where-Object { $_.Line -eq $line }) -and ($cmakeHelp -match [regex]::Escape($cand))) {
            $Generator = $cand
            break
        }
    }
}
if (-not $Generator) {
    throw "Could not pick a CMake generator. Installed VS lines: $($installs.Line -join ', '). Either upgrade CMake or pass -Generator ""<name>"" explicitly (see: cmake --help)."
}
Info "generator : $Generator"

# ------------------------------------------------------------------ rust ---
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo not found on PATH. Install Rust from https://rustup.rs"
}
$defaultTc = ''
try { $defaultTc = (& rustup default 2>&1) -join ' ' } catch { }
if ($defaultTc -notmatch 'msvc') {
    if ($NoRustupDefault) {
        throw "rustup has no usable default toolchain (got: '$defaultTc'). Run: rustup default stable-x86_64-pc-windows-msvc"
    }
    Step "rustup has no default toolchain - setting stable-x86_64-pc-windows-msvc"
    & rustup default stable-x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "rustup default failed" }
}
Info "rustc : $(& rustc --version)"
Info "cargo : $(& cargo --version)"
# NOTE: `rustc -vV` prints ~8 lines. In PowerShell, -match/-notmatch against an
# ARRAY is a filter (it returns the matching/non-matching elements), not a
# boolean -- so `(& rustc -vV) -notmatch '...'` is always truthy. Join first.
$rustcVV = (& rustc -vV) -join "`n"
if ($rustcVV -notmatch 'host:\s*x86_64-pc-windows-msvc') {
    $hostLine = ($rustcVV -split "`n" | Where-Object { $_ -match '^host:' }) -join ''
    throw "Active Rust toolchain is not x86_64-pc-windows-msvc (got '$hostLine'). The C++ static lib is MSVC-built and will not link against a GNU toolchain. Fix with: rustup default stable-x86_64-pc-windows-msvc"
}

# ----------------------------------------------------------------- b1sdk ---
$sdkSrc   = Join-Path $root 'b1sdk'
$sdkBuild = Join-Path $sdkSrc 'build'

# b1sdk/src/SDK is a copy of the Dumper-7 output that already lives in the repo
# root, so it is generated here rather than committed twice (1561 files, ~70 MB).
# b1sdk/sdk-patch/ holds the handful of files that have to be changed on top.
$sdkTree = Join-Path $sdkSrc 'src\SDK'
if (-not (Test-Path $sdkTree)) {
    Step "populating b1sdk\src\SDK from the Dumper-7 output"
    $dump = Get-ChildItem -Path $root -Directory -Filter '*+++UE5+*' |
            Select-Object -First 1
    if (-not $dump) { throw "no Dumper-7 output folder (*+++UE5+*) found in $root" }
    $cpp = Join-Path $dump.FullName 'CppSDK'
    if (-not (Test-Path $cpp)) { throw "no CppSDK inside $($dump.FullName)" }
    Copy-Item $cpp $sdkTree -Recurse -Force
    Info ("copied {0} files from {1}" -f (Get-ChildItem $sdkTree -Recurse -File).Count, $dump.Name)

    $patch = Join-Path $sdkSrc 'sdk-patch'
    Get-ChildItem $patch -Recurse -File | Where-Object { $_.Name -ne 'README.md' } | ForEach-Object {
        $rel = $_.FullName.Substring($patch.Length).TrimStart('\')
        $dst = Join-Path $sdkTree $rel
        New-Item -ItemType Directory -Force -Path (Split-Path $dst) | Out-Null
        Copy-Item $_.FullName $dst -Force
        Info "patched $rel"
    }
}
if ($Clean -and (Test-Path $sdkBuild)) { Remove-Item -Recurse -Force $sdkBuild }

Step "configuring b1sdk"
$cmArgs = @('-S', $sdkSrc, '-B', $sdkBuild, '-G', $Generator, '-A', 'x64')
if ($Full)    { $cmArgs += '-DB1SDK_FULL=ON' }
if ($Asserts) { $cmArgs += '-DB1SDK_ASSERTS=ON' }
& cmake @cmArgs
if ($LASTEXITCODE -ne 0) { throw "cmake configure failed" }

Step "building b1sdk.lib (Release) - the slow part"
$sw = [Diagnostics.Stopwatch]::StartNew()
& cmake --build $sdkBuild --config Release --parallel
if ($LASTEXITCODE -ne 0) { throw "cmake build failed" }
Info ("done in {0:N1} min" -f $sw.Elapsed.TotalMinutes)

$lib = Join-Path $sdkBuild 'Release\b1sdk.lib'
if (-not (Test-Path $lib)) { throw "b1sdk.lib was not produced at $lib" }

# build.rs looks for it at ./target/b1sdk.lib
$targetDir = Join-Path $root 'target'
New-Item -ItemType Directory -Force -Path $targetDir | Out-Null
Copy-Item $lib (Join-Path $targetDir 'b1sdk.lib') -Force
Info ("b1sdk.lib -> target\b1sdk.lib ({0:N1} MB)" -f ((Get-Item $lib).Length / 1MB))

# ------------------------------------------------------------------ dll ---
Step "building wukong_minimap.dll (cargo --release)"
Push-Location $root
try {
    & cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally { Pop-Location }

$dll = Join-Path $root 'target\release\wukong_minimap.dll'
if (-not (Test-Path $dll)) { throw "wukong_minimap.dll was not produced" }
Write-Host ("`n  OK: {0} ({1:N1} MB)" -f $dll, ((Get-Item $dll).Length / 1MB)) -ForegroundColor Green

# -------------------------------------------------------------- install ---
if ($Install) {
    if (-not (Test-Path $Install)) { throw "install path not found: $Install" }
    Step "installing into $Install"
    foreach ($n in 'wukong_minimap.dll', 'wukong_minimap.log') {
        $old = Join-Path $Install $n
        if (Test-Path $old) { Copy-Item $old "$old.bak" -Force -ErrorAction SilentlyContinue }
    }
    Copy-Item $dll (Join-Path $Install 'wukong_minimap.dll') -Force
    Copy-Item (Join-Path $root 'dist\dwmapi.dll') (Join-Path $Install 'dwmapi.dll') -Force
    $mapsDst = Join-Path $Install 'maps'
    New-Item -ItemType Directory -Force -Path $mapsDst | Out-Null
    Copy-Item (Join-Path $root 'maps\*') $mapsDst -Force -Recurse
    Info "copied wukong_minimap.dll, dwmapi.dll and maps\"
    Write-Host "`n  Launch the game, wait ~15s, then read wukong_minimap.log next to the dll." -ForegroundColor Yellow
    Write-Host "  Look for the [b1sdk] lines: GObjects/AppendString must say (scanned)," -ForegroundColor Yellow
    Write-Host "  and UWorld::GetWorld() must be non-null with a sane world name." -ForegroundColor Yellow
}

# -------------------------------------------------------------- package ---
if ($Package) {
    Step "packaging"
    $version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
    $stage = Join-Path $root "dist\wukong-minimap-$version"
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force -Path $stage | Out-Null

    Copy-Item $dll (Join-Path $stage 'wukong_minimap.dll')
    Copy-Item (Join-Path $root 'dist\dwmapi.dll') (Join-Path $stage 'dwmapi.dll')
    New-Item -ItemType Directory -Force -Path (Join-Path $stage 'maps') | Out-Null
    Copy-Item (Join-Path $root 'maps\*') (Join-Path $stage 'maps') -Recurse
    Copy-Item (Join-Path $root 'README.md') (Join-Path $stage 'README.md')

    $zip = "$stage.zip"
    if (Test-Path $zip) { Remove-Item -Force $zip }
    Compress-Archive -Path "$stage\*" -DestinationPath $zip
    Remove-Item -Recurse -Force $stage

    Info ("{0} ({1:N1} MB)" -f $zip, ((Get-Item $zip).Length / 1MB))
    Write-Host "`n  Unzip into <game>\b1\Binaries\Win64\" -ForegroundColor Yellow
}
