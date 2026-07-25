# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

<#
.SYNOPSIS
    Sets the version for the PET workspace and all crates.

.DESCRIPTION
    Updates the workspace version in the root Cargo.toml. All sub-crates inherit
    this version via `version.workspace = true`.

    For nightly/pre-release builds, pass a pre-release suffix. The base version is
    read from Cargo.toml automatically (or overridden with -Version). Use a
    deterministic pipeline variable (e.g., Build.BuildId) as the suffix to ensure
    all platform builds in the same pipeline run get the same version number.

.PARAMETER Version
    Optional base SemVer version (e.g., "0.2.0"). If omitted, reads the current
    version from the workspace Cargo.toml.

.PARAMETER Suffix
    Optional pre-release suffix appended as "-<Suffix>" (e.g., "dev.12345").
    For nightly builds, use the Azure Pipelines Build.BuildId to guarantee
    all platforms in the same run get an identical version.

.EXAMPLE
    # Stable release — explicit version
    ./set-version.ps1 -Version 1.0.0

.EXAMPLE
    # Nightly build — reads base version from Cargo.toml, appends suffix
    ./set-version.ps1 -Suffix "dev.$(Build.BuildId)"
#>

param(
    [Parameter(Mandatory = $false)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version,

    [Parameter(Mandatory = $false)]
    [ValidatePattern('^[a-zA-Z0-9._-]+$')]
    [string]$Suffix
)

$ErrorActionPreference = 'Stop'

$cargoToml = Join-Path $PSScriptRoot 'Cargo.toml'
$content = Get-Content $cargoToml -Raw

# NOTE: Assumes `version` is the first key after [workspace.package] header.
$pattern = '(?m)(^\[workspace\.package\]\s*\r?\nversion\s*=\s*)"([^"]*)"'
if ($content -notmatch $pattern) {
    Write-Error "Could not find [workspace.package] version in $cargoToml"
    exit 1
}

# Read current version from Cargo.toml if -Version not provided
if (-not $Version) {
    $Version = $Matches[2]
    # Strip any existing pre-release suffix to get the base version
    $Version = ($Version -split '-')[0]
    Write-Host "Read base version from Cargo.toml: $Version"
}

if ($Suffix) {
    $fullVersion = "$Version-$Suffix"
} else {
    $fullVersion = $Version
}

Write-Host "Setting PET version to: $fullVersion"

$content = $content -replace $pattern, "`${1}`"$fullVersion`""
Set-Content $cargoToml $content -NoNewline -Encoding utf8

Write-Host "Updated $cargoToml"
Write-Host "Version set to: $fullVersion"
