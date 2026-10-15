# World of Skatecraft setup (Windows, experimental). Double-click setup.bat, or from PowerShell:
#
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1                 # everything
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -Wow DIR -Skate DIR
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -Server OLD\server
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -Doctor         # only check this PC
#
# Every step is skipped when its result is already there, so after a failure fix the reported
# problem and run it again. Everything is logged to setup.log. Mirrors setup/setup.sh.
param(
    [string]$Wow = "",
    [string]$Skate = "",
    [string]$Server = "",
    [switch]$Doctor
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = '1' }

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$State = Join-Path $Root '.setup'
New-Item -ItemType Directory -Force -Path $State | Out-Null
Start-Transcript -Path (Join-Path $Root 'setup.log') -Append | Out-Null

# Pinned upstreams, the same as setup/setup.sh.
$EngineUrl = 'https://github.com/SK8-ENGINE/skate-3-rust-engine'
$EngineRev = 'cb79689'
$ConverterUrl = 'https://raw.githubusercontent.com/Kimmo3223/2010-rust-rewrite-mashup/adb684e78ec07654f00e2a9da88d32ae02cc68bd/skate/converter/iw4l_skate_convert.py'
$VgmstreamUrl = 'https://github.com/vgmstream/vgmstream/releases/download/r2117/vgmstream-win64.zip'
$ServerUrl = 'https://github.com/mserajnik/vmangos-deploy'
$ServerRev = 'df0f03e9f4a826980001cbb047a00301145744ce'
if (-not $env:SKATECRAFT_SERVER_PROJECT) { $env:SKATECRAFT_SERVER_PROJECT = 'world-of-skatecraft' }

function Step($text) { Write-Host ''; Write-Host "==> $text" -ForegroundColor Cyan }
function Stop-Setup($message, $fix = '') {
    Write-Host ''
    Write-Host "SETUP STOPPED: $message" -ForegroundColor Red
    if ($fix) { Write-Host "Fix: $fix" }
    Write-Host 'Then run setup again; finished steps are skipped.'
    Stop-Transcript | Out-Null
    exit 1
}
# Runs a program and stops setup if it fails (PowerShell 5 does not stop on exit codes).
function Invoke-Native($what, [scriptblock]$command) {
    & $command
    if ($LASTEXITCODE -ne 0) { Stop-Setup "$what failed (see the lines above)" }
}
function Test-Native([scriptblock]$command) {
    try { & $command *> $null; return $LASTEXITCODE -eq 0 } catch { return $false }
}

# ── Doctor ─────────────────────────────────────────────────────────────────────────────────────
function Test-Machine {
    Step 'Checking this PC'
    $script:bad = $false
    function Check($name, [bool]$ok, $fix) {
        if ($ok) { Write-Host "  ok       $name" }
        else { Write-Host "  MISSING  $name" -ForegroundColor Yellow; Write-Host "           fix: $fix"; $script:bad = $true }
    }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $msvc = (Test-Path $vswhere) -and
        [bool](& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    Check 'Rust (rustup/cargo)' ([bool](Get-Command cargo -ErrorAction SilentlyContinue)) `
        'winget install -e --id Rustlang.Rustup   (then open a new terminal)'
    Check 'Visual Studio C++ build tools' $msvc `
        'winget install -e --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
    Check 'git' ([bool](Get-Command git -ErrorAction SilentlyContinue)) 'winget install -e --id Git.Git'
    Check 'Python 3 with venv' (Test-Native { python -c 'import venv, ensurepip' }) `
        'winget install -e --id Python.Python.3.12   (if "python" opens the Microsoft Store, turn off its App execution alias in Settings)'
    Check 'Docker Desktop' ([bool](Get-Command docker -ErrorAction SilentlyContinue)) 'winget install -e --id Docker.DockerDesktop   (then restart Windows)'
    Check 'Docker compose' (Test-Native { docker compose version }) 'update Docker Desktop'
    Check 'Docker Desktop running' (Test-Native { docker info }) 'start Docker Desktop and wait until it says it is running'
    $drive = (Split-Path -Qualifier $Root).TrimEnd(':')
    $free = [math]::Floor((Get-PSDrive $drive).Free / 1GB)
    Check "20 GB free disk ($free GB)" ($free -ge 20) 'free up disk space'
    $long = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -ErrorAction SilentlyContinue).LongPathsEnabled
    if ($long -ne 1 -and $Root.Length -gt 40) {
        Write-Host "  note     the repo path is long and Windows long paths are off; if the build fails with 'path too long', move the repo to something short like C:\skatecraft"
    }
    return -not $script:bad
}

if ($Doctor) {
    if (Test-Machine) { Write-Host 'All good.'; Stop-Transcript | Out-Null; exit 0 }
    Stop-Transcript | Out-Null
    exit 1
}
if (-not (Test-Machine)) { Stop-Setup 'this PC is missing the tools listed above' 'install them, open a new terminal, run setup again' }

function Read-Folder($value, $prompt) {
    while (-not $value) { $value = Read-Host $prompt }
    $value = $value.Trim().Trim('"')
    return [System.IO.Path]::GetFullPath($value)
}

# ── WoW client ─────────────────────────────────────────────────────────────────────────────────
$WowLink = Join-Path $Root 'WoW'
if (-not (Test-Path (Join-Path $WowLink 'Data'))) {
    Step 'WoW 1.12.1 client'
    $Wow = Read-Folder $Wow 'Path to your WoW 1.12.1 (English, build 5875) folder, the one with Data in it'
    if ((Split-Path -Leaf $Wow) -eq 'Data') { $Wow = Split-Path -Parent $Wow }
    foreach ($mpq in 'dbc.MPQ', 'patch.MPQ', 'model.MPQ', 'terrain.MPQ') {
        if (-not (Test-Path (Join-Path $Wow "Data\$mpq"))) {
            Stop-Setup "$Wow\Data has no $mpq" 'point setup at an untouched English 1.12.1 client folder'
        }
    }
    if (Test-Path $WowLink) { (Get-Item $WowLink).Delete() }
    New-Item -ItemType Junction -Path $WowLink -Target $Wow | Out-Null
    Set-Content (Join-Path $State 'wow-path') $Wow
    Write-Host "  linked WoW -> $Wow"
}

# ── Skate 3 data ───────────────────────────────────────────────────────────────────────────────
$SkateFile = Join-Path $State 'skate3-path'
if (-not $Skate -and (Test-Path $SkateFile)) { $Skate = (Get-Content $SkateFile -Raw).Trim() }
function Get-Skate3 {
    $script:Skate = Read-Folder $script:Skate 'Path to your extracted Skate 3 (Xbox 360) folder, the one with default.xex and data'
    if (-not ((Test-Path (Join-Path $script:Skate 'default.xex')) -and (Test-Path (Join-Path $script:Skate 'data')))) {
        Stop-Setup "$script:Skate has no default.xex with a data folder beside it" `
            'extract your Skate 3 disc (for example with extract-xiso) and point setup at that folder; ISO files do not work'
    }
    Set-Content $SkateFile $script:Skate
}
$Venv = Join-Path $State 'venv\Scripts\python.exe'
function Initialize-Venv {
    if (-not (Test-Path $Venv)) { Invoke-Native 'creating a Python venv' { python -m venv (Join-Path $State 'venv') } }
    Invoke-Native 'pip' { & $Venv -m pip install --quiet --upgrade pip }
    Invoke-Native 'installing numpy and Pillow' { & $Venv -m pip install --quiet 'numpy>=2.2,<3' 'Pillow>=11.3' }
}

if (-not (Test-Path (Join-Path $Root 'skate-data\assets\private\skater.glb'))) {
    Step 'Converting Skate 3 data (skater, animations, physics)'
    Get-Skate3
    $Engine = Join-Path $State 'skate-engine'
    if (-not (Test-Path (Join-Path $Engine '.git'))) { Invoke-Native 'cloning the skate engine' { git clone --quiet $EngineUrl $Engine } }
    & git -C $Engine fetch --quiet origin
    Invoke-Native 'checking out the skate engine' { git -C $Engine checkout --quiet $EngineRev }
    Initialize-Venv
    # The converters look for this exact file name.
    New-Item -ItemType Directory -Force -Path (Join-Path $Engine 'target\native') | Out-Null
    Invoke-Native 'building the RefPack decoder' {
        rustc --edition 2024 --crate-type cdylib -C opt-level=3 -C panic=abort `
            (Join-Path $Engine 'tools\asset_pipeline\refpack_native.rs') -o (Join-Path $Engine 'target\native\refpack.dll')
    }
    Invoke-WebRequest -UseBasicParsing $ConverterUrl -OutFile (Join-Path $Engine 'iw4l_skate_convert.py')
    Invoke-Native 'the Skate 3 conversion' {
        & $Venv (Join-Path $Engine 'iw4l_skate_convert.py') --xex (Join-Path $Skate 'default.xex') --out (Join-Path $Root 'skate-data')
    }
    if (-not (Test-Path (Join-Path $Root 'skate-data\assets\private\skater.glb'))) { Stop-Setup 'the conversion finished without skater.glb' }
}

# ── Skate 3 sounds ─────────────────────────────────────────────────────────────────────────────
if (-not (Test-Path (Join-Path $Root 'skate-audio\pop_1.wav'))) {
    Step 'Converting Skate 3 sounds'
    Get-Skate3
    Initialize-Venv
    $Vgm = Join-Path $State 'vgmstream'
    if (-not (Test-Path (Join-Path $Vgm 'vgmstream-cli.exe'))) {
        $zip = Join-Path $State 'vgmstream.zip'
        Invoke-WebRequest -UseBasicParsing $VgmstreamUrl -OutFile $zip
        Expand-Archive -Force $zip $Vgm
    }
    $env:Path = "$Vgm;$env:Path"
    & $Venv (Join-Path $Root 'tools\skate_audio.py') (Join-Path $Skate 'data') (Join-Path $Root 'skate-audio')
    if ($LASTEXITCODE -ne 0) { Write-Host '  (sounds failed; skating still works, just silent)' }
}

# ── Local server ───────────────────────────────────────────────────────────────────────────────
$ServerArgs = @()
if ($Server) { $ServerArgs = @('--server', $Server) }
$Server = & python (Join-Path $Root 'setup\migrate_database.py') path @ServerArgs
if ($LASTEXITCODE -ne 0) { Stop-Setup 'could not select the existing server' }
$Server = ([string]$Server).Trim()
Write-Host "  Using server: $Server"
$Compose = Join-Path $Server 'compose.yaml'
function Invoke-Compose { & docker compose -p $env:SKATECRAFT_SERVER_PROJECT --project-directory $Server -f $Compose @args }
if (-not (Test-Path $Compose)) {
    Step 'Setting up the local vmangos server'
    if (-not (Test-Path (Join-Path $Server '.git'))) { Invoke-Native 'cloning vmangos-deploy' { git clone --quiet $ServerUrl $Server } }
    if (-not (Test-Native { git -C $Server cat-file -e $ServerRev })) {
        Invoke-Native 'fetching the pinned deployment source' { git -C $Server fetch --quiet origin $ServerRev }
    }
    Invoke-Native 'checking out vmangos-deploy' { git -C $Server checkout --quiet $ServerRev }
    foreach ($name in 'mangosd', 'realmd') {
        $ConfigPath = Join-Path $Server "config\$name.conf"
        if (-not (Test-Path $ConfigPath)) { Copy-Item (Join-Path $Server "config\$name.conf.example") $ConfigPath }
    }
    Copy-Item (Join-Path $Server 'compose.yaml.example') $Compose
}
Step 'Backing up and upgrading the server database (existing characters are kept)'
Invoke-Native 'database migration' { python (Join-Path $Root 'setup\migrate_database.py') upgrade --server $Server }
$MapsReady = & python (Join-Path $Root 'setup\migrate_database.py') maps-ready --server $Server
if ($LASTEXITCODE -ne 0) { Stop-Setup 'could not check the existing map data' }
if ($MapsReady -ne '1') {
    Step 'Extracting server maps from your WoW client (this takes HOURS; leave it running)'
    $WowDir = (Get-Item $WowLink).Target | Select-Object -First 1
    if (-not $WowDir) { $WowDir = (Get-Content (Join-Path $State 'wow-path') -Raw).Trim() }
    Invoke-Native 'map extraction' {
        python (Join-Path $Root 'setup\migrate_database.py') extract --server $Server --wow $WowDir
    }
}
Step 'Starting the server'
Invoke-Native 'starting the server' { Invoke-Compose up -d --wait --wait-timeout 600 }
Invoke-Native 'verifying the patched server and database' { python (Join-Path $Root 'setup\migrate_database.py') check --live --server $Server }

# ── Account ────────────────────────────────────────────────────────────────────────────────────
$AccountFile = Join-Path $State 'account'
$ExistingAccounts = & python (Join-Path $Root 'setup\migrate_database.py') accounts --server $Server
if ($LASTEXITCODE -ne 0) { Stop-Setup 'could not check existing accounts' }
if ([int]$ExistingAccounts -gt 0) {
    Write-Host '  Existing accounts and characters were kept. Log in with your existing username and password.'
} else {
    Step 'Game account'
    $account = Read-Host 'Account name (letters and digits)'
    $secret = Read-Host 'Password' -AsSecureString
    $password = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret))
    Invoke-Native 'creating the account' { python (Join-Path $Root 'tools\make_account.py') $account $password --server $Server }
    Set-Content $AccountFile $account
}

# ── Build ──────────────────────────────────────────────────────────────────────────────────────
Step 'Building the game (first time: several minutes)'
Invoke-Native 'preparing the skate camera host' { python (Join-Path $Root 'setup\prepare_skate_engine.py') }
Invoke-Native 'the build' { cargo build --release -p benilla }

Step 'Done'
Write-Host '  Start the game with:  play.bat'
$PlayerFile = Join-Path $State 'player-account.json'
if (Test-Path $PlayerFile) {
    $Player = Get-Content $PlayerFile -Raw | ConvertFrom-Json
    if ($Player.character) { Write-Host "  Your character $($Player.character) loads automatically. Press J to skate." }
    else { Write-Host '  Login is automatic. Create a character, then press J to skate.' }
} elseif (Test-Path $AccountFile) {
    Write-Host "  Log in as $((Get-Content $AccountFile -Raw).Trim()), create a character, press J to skate."
} else {
    Write-Host '  Log in with your existing account and choose your character. Missing skate abilities are learned at login.'
}
if (Test-Path (Join-Path $State 'steam-game.json')) { Write-Host '  PlayStation controller support is configured through Steam Input.' }
Stop-Transcript | Out-Null
