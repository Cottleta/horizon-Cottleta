# Puts this folder's Horizon build into the Horizon Launcher as its own version and makes the
# default instance play it. Needs the launcher installed and the game launched once, so the
# files the game prepares from Minecraft are there to copy.
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$tag = (Get-Content -Raw (Join-Path $here 'tag.txt')).Trim()
$launcher = Join-Path $env:APPDATA 'horizon-launcher'
$versions = Join-Path $launcher 'versions'
$target = Join-Path $versions $tag

function Stop-With($message) {
    Write-Host ''
    Write-Host $message -ForegroundColor Red
    exit 1
}
# A UTF-8 file without the byte order mark Windows PowerShell's own writers add: the
# launcher's and the game's readers don't skip one.
function Write-Text($path, $text) {
    [IO.File]::WriteAllText($path, $text, (New-Object Text.UTF8Encoding($false)))
}

foreach ($file in 'horizon-client.exe', 'horizon-ctl.exe') {
    if (-not (Test-Path (Join-Path $here $file))) {
        Stop-With "$file is missing. Unpack the whole zip into a folder first, then run this from there."
    }
}
if (Get-Process -Name 'horizon-launcher', 'horizon-client' -ErrorAction SilentlyContinue) {
    Stop-With 'Close the Horizon Launcher and the game first, then run this again.'
}
if (-not (Test-Path $versions)) {
    Stop-With 'The Horizon Launcher is not set up yet. Install it, open it, launch the game once, close it, then run this again.'
}
# The date of a version: a nightly's from its name, any other's from its folder.
function Get-VersionDate($version) {
    if ($version.Name -match '^nightly-(\d{4}-\d{2}-\d{2})') {
        return $Matches[1]
    }
    return $version.LastWriteTime.ToString('yyyy-MM-dd')
}
# The newest official version whose game has run: its prepared files come along. This build
# was tested on files prepared by the $oldestData nightly, so none older will do.
$oldestData = '2026-11-01'
$source = Get-ChildItem $versions -Directory |
    Where-Object {
        $resources = Join-Path $_.FullName '.local\horizon-client\26.3'
        $_.Name -notlike '*Cottleta*' -and
        (Get-VersionDate $_) -ge $oldestData -and
        (Test-Path (Join-Path $_.FullName '.installed')) -and
        (Test-Path (Join-Path $resources 'horizon-resources.json')) -and
        (Test-Path (Join-Path $resources 'assets\minecraft\sounds.json'))
    } |
    Sort-Object { Get-VersionDate $_ }, LastWriteTime -Descending |
    Select-Object -First 1
if (-not $source) {
    Stop-With 'No recent version has run yet. Open the launcher, let it update, press Play on the newest version once, close the game and the launcher, then run this again.'
}

Write-Host "Copying $($source.Name) to $tag (about 1.5 GB, a minute or so)..."
if (Test-Path $target) {
    Remove-Item -Recurse -Force $target
}
# robocopy's exit codes below 8 mean success.
robocopy $source.FullName $target /E /NFL /NDL /NJH /NJS /NP | Out-Null
if ($LASTEXITCODE -ge 8) {
    Stop-With "Copying failed (robocopy code $LASTEXITCODE). Is the disk full?"
}
foreach ($file in 'horizon-client.exe', 'horizon-ctl.exe') {
    Copy-Item -Force (Join-Path $here $file) (Join-Path $target $file)
    $wanted = (Get-FileHash (Join-Path $here $file)).Hash
    if ((Get-FileHash (Join-Path $target $file)).Hash -ne $wanted) {
        Stop-With "$file did not copy correctly."
    }
}

# Pin the default instance to this version: the launcher then neither updates nor removes it.
$instancesFile = Join-Path $launcher 'instances.json'
$instances = if (Test-Path $instancesFile) {
    Copy-Item -Force $instancesFile "$instancesFile.before-$tag"
    Get-Content -Raw $instancesFile | ConvertFrom-Json
} else {
    New-Object PSObject
}
if (-not $instances.PSObject.Properties['instances']) {
    $instances | Add-Member instances (New-Object PSObject)
}
if (-not $instances.instances.PSObject.Properties['horizon']) {
    $instances.instances | Add-Member horizon (New-Object PSObject)
}
$instances.instances.horizon | Add-Member -Force version $tag
Write-Text $instancesFile ($instances | ConvertTo-Json -Depth 20)

# The bundled graphics settings. The old file stays next to it.
$bundled = Join-Path $here 'horizon-graphics.txt'
if (Test-Path $bundled) {
    $game = Join-Path $env:APPDATA 'horizon'
    New-Item -ItemType Directory -Force $game | Out-Null
    $settings = Join-Path $game 'horizon-graphics.txt'
    if (Test-Path $settings) {
        Copy-Item -Force $settings "$settings.before-$tag"
    }
    Write-Text $settings (Get-Content -Raw $bundled)
}

Write-Host ''
Write-Host "Done. Open the launcher and press Play: the default instance now runs $tag." -ForegroundColor Green
Write-Host "To go back: in the launcher, set that instance's version to another one."
