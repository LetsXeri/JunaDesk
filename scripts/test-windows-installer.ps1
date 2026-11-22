param(
    [Parameter(Mandatory)]
    [string] $Installer
)

$ErrorActionPreference = 'Stop'

function Assert-ValidSignature([string] $Path) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid' -or !$signature.TimeStamperCertificate) {
        throw "Invalid or untimestamped Authenticode signature for ${Path}: $($signature.StatusMessage)"
    }
}

function Invoke-Setup([string] $Path, [string] $Arguments) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "$Path exited with $($process.ExitCode)." }
}

$installerPath = (Resolve-Path -LiteralPath $Installer).Path
if ($env:SIGN_WINDOWS -eq "true") { Assert-ValidSignature $installerPath }
$installDir = Join-Path $env:RUNNER_TEMP 'JunaDesk-InstallTest'

$appPath = Join-Path $installDir 'junadesk.exe'
$uninstaller = Join-Path $installDir 'uninstall.exe'
$previousSetup = Join-Path $env:RUNNER_TEMP 'JunaDesk-v0.1.3-Windows-x64.exe'
Invoke-WebRequest -Uri 'https://github.com/LetsXeri/JunaDesk/releases/download/v0.1.3/JunaDesk-v0.1.3-Windows-x64.exe' -OutFile $previousSetup
$previousSha256 = '2ff844ed64b7fa5bf523578ff47b64864811039eb8f56414e8eeb2a1f36a6a1a'
if ((Get-FileHash -LiteralPath $previousSetup -Algorithm SHA256).Hash -ne $previousSha256) {
    throw 'The v0.1.3 upgrade fixture does not match the published installer.'
}
Invoke-Setup $previousSetup "/S /D=$installDir"
if (!(Test-Path -LiteralPath $appPath -PathType Leaf)) { throw 'v0.1.3 was not installed.' }
Invoke-Setup $installerPath "/S /UPDATE /D=$installDir"

foreach ($path in @('junadesk.exe', 'uninstall.exe', 'tools/ffmpeg.exe', 'tools/ffprobe.exe')) {
    $file = Join-Path $installDir $path
    if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Setup did not install $path." }
    if ($env:SIGN_WINDOWS -eq 'true') { Assert-ValidSignature $file }
}
if ((Test-Path -LiteralPath (Join-Path $installDir 'tools/mpv.exe')) -or
    (Get-ChildItem -LiteralPath (Join-Path $installDir 'tools') -File -Filter '*.dll')) {
    throw 'The installation still contains legacy media binaries.'
}
& (Join-Path $installDir 'tools/ffmpeg.exe') -version
if ($LASTEXITCODE -ne 0) { throw 'Installed FFmpeg could not start.' }

Invoke-Setup $uninstaller '/S'
if ((Test-Path -LiteralPath $appPath) -or (Test-Path -LiteralPath $uninstaller)) {
    throw 'Uninstaller left JunaDesk installed.'
}
