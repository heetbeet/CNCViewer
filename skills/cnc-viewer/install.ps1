[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$viewer = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../output/desktop/CNC Viewer.exe'))
if (!(Test-Path -LiteralPath $viewer -PathType Leaf)) {
    throw 'Build the native viewer with tools/build.ps1 before installing its skill.'
}
$destination = Join-Path $env:USERPROFILE '.codex/skills/cnc-viewer'
if (Test-Path -LiteralPath $destination) {
    $existing = Get-Item -LiteralPath $destination
    if ($existing.LinkType) {
        throw 'The skill destination must be a local directory, not a link.'
    }
}
New-Item -ItemType Directory -Force -Path (Join-Path $destination 'agents') | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'SKILL.md') -Destination (Join-Path $destination 'SKILL.md') -Force
[IO.File]::WriteAllText((Join-Path $destination 'executable.txt'), $viewer, [Text.UTF8Encoding]::new($false))
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'agents/openai.yaml') -Destination (Join-Path $destination 'agents/openai.yaml') -Force
