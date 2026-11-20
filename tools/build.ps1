$ErrorActionPreference = 'Stop'
Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    cargo build --locked --release -p cnc-viewer
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed.' }
    $desktop = (New-Item -ItemType Directory -Force 'output/desktop').FullName
    $build = (Resolve-Path -LiteralPath 'target').Path
    $destination = Join-Path $desktop 'CNC Viewer.exe'
    $id = [guid]::NewGuid().ToString('N')
    $incoming = Join-Path $desktop "cnc-viewer-incoming-$id.tmp"
    $retired = Join-Path $build "cnc-viewer-retired-$id.exe"
    $lock = [IO.File]::Open((Join-Path $build 'package.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
    try {
        Get-ChildItem -LiteralPath $build -Filter 'cnc-viewer-retired-*.exe' -File |
            ForEach-Object { Remove-Item -LiteralPath $_.FullName -ErrorAction SilentlyContinue }
        Copy-Item -LiteralPath 'target/release/cnc-viewer.exe' -Destination $incoming
        if (Test-Path -LiteralPath $destination) {
            # Windows can move a running executable but cannot overwrite its mapped image.
            [IO.File]::Move($destination, $retired)
        }
        try {
            [IO.File]::Move($incoming, $destination)
        } catch {
            if (!(Test-Path -LiteralPath $destination) -and (Test-Path -LiteralPath $retired)) {
                [IO.File]::Move($retired, $destination)
            }
            throw
        }
    } finally {
        Remove-Item -LiteralPath $incoming -ErrorAction SilentlyContinue
        if (Test-Path -LiteralPath $destination) {
            Remove-Item -LiteralPath $retired -ErrorAction SilentlyContinue
        }
        $lock.Dispose()
    }
    Get-Item -LiteralPath $destination | Select-Object FullName, Length
} finally { Pop-Location }
