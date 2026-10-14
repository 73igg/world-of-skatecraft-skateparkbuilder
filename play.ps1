param(
    [switch]$Check,
    [switch]$FromSteam,
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$GameArguments
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-Location -LiteralPath $PSScriptRoot

try {
    $gameExe = Join-Path $PSScriptRoot 'target\release\benilla.exe'
    if (-not (Test-Path -LiteralPath $gameExe)) {
        throw 'The game executable is missing. Run setup.bat first.'
    }
    if (-not (Test-Path -LiteralPath '.setup\probe-launch.json')) {
        $existingGame = Get-Process benilla -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $gameExe }
        if ($existingGame) {
            Write-Host 'World of Skatecraft is already running. Select its window on the taskbar.'
            exit 0
        }
    }

    $serverDataFolder = 'server\storage\mangosd\extracted-data'
    $stagedDataMarker = Join-Path $PSScriptRoot '.setup\play-data-active'
    if (Test-Path -LiteralPath $stagedDataMarker) {
        $serverDataFolder = 'server\storage\mangosd\play-data'
        if ((Test-Path -LiteralPath '.setup\maps-ready') -and (Test-Path -LiteralPath '.setup\pathfinding-verified')) {
            $finalDataFolder = Join-Path $PSScriptRoot 'server\storage\mangosd\extracted-data'
            foreach ($folder in 'maps', 'vmaps', 'mmaps', '5875\dbc') {
                if (-not (Get-ChildItem -LiteralPath (Join-Path $finalDataFolder $folder) -File -ErrorAction SilentlyContinue | Select-Object -First 1)) {
                    throw 'The completed server data is missing files.'
                }
            }
            $composePath = Join-Path $PSScriptRoot 'server\compose.yaml'
            $composeContent = [IO.File]::ReadAllText($composePath)
            [IO.File]::WriteAllText($composePath, $composeContent.Replace('./storage/mangosd/play-data:/opt/vmangos/storage/data:ro', './storage/mangosd/extracted-data:/opt/vmangos/storage/data:ro'), (New-Object Text.UTF8Encoding($false)))
            $serverConfigPath = Join-Path $PSScriptRoot 'server\config\mangosd.conf'
            $serverConfig = [IO.File]::ReadAllText($serverConfigPath)
            [IO.File]::WriteAllText($serverConfigPath, ($serverConfig -replace '(?m)^mmap.enabled = 0\r?$', 'mmap.enabled = 1'), (New-Object Text.UTF8Encoding($false)))
            Remove-Item -LiteralPath $stagedDataMarker
            $serverDataFolder = 'server\storage\mangosd\extracted-data'
        }
    }
    $requiredAssets = @('WoW\Data', 'skate-data\assets\rig.json', 'skate-data\assets\board.json', 'skate-audio\pop_1.wav', 'server\compose.yaml')
    $requiredAssets += @('maps', 'vmaps', 'mmaps', '5875\dbc') | ForEach-Object { Join-Path $serverDataFolder $_ }
    foreach ($asset in $requiredAssets) {
        if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot $asset))) {
            if ($asset.StartsWith('server\storage\mangosd\')) {
                throw 'The server maps are still being prepared. Launch again after setup finishes.'
            }
            throw 'Game setup is incomplete. Run setup.bat first.'
        }
    }

    $dockerCommand = Get-Command docker.exe -ErrorAction SilentlyContinue
    $dockerExe = if ($dockerCommand) { $dockerCommand.Source } else { $null }
    $desktopFolders = @(
        (Join-Path $env:LOCALAPPDATA 'Programs\DockerDesktop'),
        (Join-Path $env:ProgramFiles 'Docker\Docker')
    )
    if (-not $dockerExe) {
        foreach ($folder in $desktopFolders) {
            $candidate = Join-Path $folder 'resources\bin\docker.exe'
            if (Test-Path -LiteralPath $candidate) { $dockerExe = $candidate; break }
        }
    }
    if (-not $dockerExe) { throw 'Docker Desktop could not be found.' }
    $env:Path = "$(Split-Path -Parent $dockerExe);$env:Path"

    function Test-GameDocker {
        try {
            & $dockerExe info --format '{{.ServerVersion}}' *> $null
            return $LASTEXITCODE -eq 0
        } catch { return $false }
    }

    Write-Host 'Starting World of Skatecraft...'
    if (-not (Test-GameDocker)) {
        Write-Host 'Starting Docker Desktop...'
        Start-Process -FilePath $dockerExe -ArgumentList 'desktop', 'start', '--detach' -WindowStyle Hidden
        $dockerDeadline = [DateTime]::UtcNow.AddMinutes(3)
        while (-not (Test-GameDocker)) {
            if ([DateTime]::UtcNow -gt $dockerDeadline) {
                throw 'Docker Desktop did not finish starting. Open Docker Desktop to check its status.'
            }
            Start-Sleep -Seconds 3
        }
    }

    $serverProject = if ($env:SKATECRAFT_SERVER_PROJECT) { $env:SKATECRAFT_SERVER_PROJECT } else { 'world-of-skatecraft' }
    Write-Host 'Starting the local server...'
    $ErrorActionPreference = 'Continue'
    & $dockerExe compose -p $serverProject --project-directory server -f server\compose.yaml up -d --wait --wait-timeout 180
    $serverStartExit = $LASTEXITCODE
    $ErrorActionPreference = 'Stop'
    if ($serverStartExit -ne 0) { throw 'The local server could not start. See the message above.' }

    if ($Check) {
        Write-Host 'World of Skatecraft is ready to launch.'
        exit 0
    }

    $parkAssetsFolder = Join-Path $PSScriptRoot 'benilla-config\park-assets'
    $env:WOW_PARK_ASSETS = $parkAssetsFolder
    $catalogPython = Join-Path $PSScriptRoot '.setup\venv\Scripts\python.exe'
    if (-not (Test-Path -LiteralPath (Join-Path $parkAssetsFolder 'skate-catalog.json'))) {
        if (-not (Test-Path -LiteralPath $catalogPython)) { throw 'The object catalog converter is missing. Run setup.bat first.' }
        Write-Host 'Preparing the Skate object catalog from your game files...'
        & $catalogPython (Join-Path $PSScriptRoot 'tools\prepare_park_catalog.py') --out $parkAssetsFolder
        if ($LASTEXITCODE -ne 0) { throw 'The object catalog could not be prepared. See the conversion message above.' }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $parkAssetsFolder 'wow-names.json')) -and (Test-Path -LiteralPath $catalogPython)) {
        & $catalogPython (Join-Path $PSScriptRoot 'tools\cache_park_object_names.py') --docker $dockerExe --container "$serverProject-database-1" --out (Join-Path $parkAssetsFolder 'wow-names.json')
        if ($LASTEXITCODE -ne 0) { throw 'The WoW object names could not be prepared.' }
    }

    $steamConfigFile = Join-Path $PSScriptRoot '.setup\steam-game.json'
    if (-not $FromSteam -and (Test-Path -LiteralPath $steamConfigFile)) {
        $steamGame = Get-Content -LiteralPath $steamConfigFile -Raw | ConvertFrom-Json
        Write-Host 'Starting the game with PlayStation controller support...'
        Start-Process -FilePath $steamGame.steam_exe -ArgumentList '-silent', "steam://rungameid/$($steamGame.game_id)" -WindowStyle Hidden
        exit 0
    }

    $env:WOW_HOST = '127.0.0.1'
    $env:WOW_SKATE_ASSETS = Join-Path $PSScriptRoot 'skate-data\assets'
    $env:WOW_SKATE_AUDIO = Join-Path $PSScriptRoot 'skate-audio'
    $env:WOW_SKATE_CAM = 'engine'
    $env:WOW_SKATE_CAM_PULL = '4.0'
    $loginFile = Join-Path $PSScriptRoot '.setup\player-account.json'
    if (Test-Path -LiteralPath $loginFile) {
        $playerLogin = Get-Content -LiteralPath $loginFile -Raw | ConvertFrom-Json
        $env:WOW_USER = $playerLogin.user
        $env:WOW_PASS = $playerLogin.password
        if ($playerLogin.character) { $env:WOW_CHAR = $playerLogin.character }
    }

    $probeLaunchFile = Join-Path $PSScriptRoot '.setup\probe-launch.json'
    if (Test-Path -LiteralPath $probeLaunchFile) {
        $probeLaunch = Get-Content -LiteralPath $probeLaunchFile -Raw | ConvertFrom-Json
        foreach ($entry in $probeLaunch.PSObject.Properties) {
            [Environment]::SetEnvironmentVariable($entry.Name, [string]$entry.Value, 'Process')
        }
        $ErrorActionPreference = 'Continue'
        & $gameExe @GameArguments *> (Join-Path $PSScriptRoot '.setup\live-check.log')
        exit $LASTEXITCODE
    }

    if (Test-Path -LiteralPath $loginFile) {
        # An interactive launch intentionally uses the player's configured account.
        $env:WOW_ALLOW_ACCOUNT = '1'
    }

    $ErrorActionPreference = 'Continue'
    & $gameExe @GameArguments
    exit $LASTEXITCODE
} catch {
    Write-Host "World of Skatecraft: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
