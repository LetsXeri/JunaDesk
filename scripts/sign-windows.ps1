param(
    [Parameter(Mandatory, Position = 0)]
    [string[]] $Path
)

$ErrorActionPreference = 'Stop'
foreach ($file in $Path) {
    $binary = (Resolve-Path -LiteralPath $file).Path
    & $env:JUNADESK_SIGNTOOL sign /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 `
        /dlib $env:JUNADESK_SIGNING_DLIB /dmdf $env:JUNADESK_SIGNING_METADATA $binary
    if ($LASTEXITCODE -ne 0) { throw "SignTool failed for $binary ($LASTEXITCODE)." }
}
