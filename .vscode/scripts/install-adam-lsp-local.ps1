# Builds and installs adam-lsp + the vscode-adam-lang extension from a copy of
# this workspace staged outside any git worktree. Building/installing directly
# from a worktree leaves the running lsp binary and extension referencing files
# inside the worktree, which locks the worktree directory (on Windows) and
# blocks `git worktree remove` until the extension is uninstalled. Staging a
# copy means only *updating* the install requires an uninstall first (to
# release the lock on the binary being replaced) — deleting the worktree never
# does.
if (-not (Get-Command robocopy -ErrorAction SilentlyContinue)) {
    Write-Error "robocopy not found. This script stages the workspace copy via robocopy and is Windows-only."
    exit 1
}
foreach ($cmd in "cargo", "npm") {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) {
        Write-Error "$cmd not found on PATH; required to build/package/install from the staged copy."
        exit 1
    }
}
if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    Write-Error "LOCALAPPDATA is not set; cannot pick a staging directory outside the worktree."
    exit 1
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$stageDir = Join-Path $env:LOCALAPPDATA "cel-rs\adam-lsp-install"

if (-not (Test-Path (Join-Path $repoRoot "adam-lsp\Cargo.toml"))) {
    Write-Error "Expected to find adam-lsp\Cargo.toml under '$repoRoot' (derived from this script's location). Is the script still at .vscode/scripts/ in the repo root?"
    exit 1
}

New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
Write-Host "Staging workspace copy at: $stageDir"

# ".git" is listed in both /XD and /XF: in a worktree it's a plain file (pointing at the
# main checkout's gitdir), which only /XF matches; in an ordinary clone it's a directory,
# which only /XD matches. Both are excluded either way, since this script may run from either.
robocopy $repoRoot $stageDir /MIR `
    /XD ".git" "target" "node_modules" ".claude" ".tokensave" `
    /XF "*.vsix" ".git" `
    /R:2 /W:2 /NFL /NDL /NJH | Out-Null
if ($LASTEXITCODE -ge 8) {
    Write-Error "robocopy failed with exit code $LASTEXITCODE"
    exit $LASTEXITCODE
}

Push-Location $stageDir
try {
    cargo install --path adam-lsp --force
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Push-Location "editors/vscode-adam-lang"
    try {
        npm ci
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

        npm run package
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

        npm run install-extension
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    } finally {
        Pop-Location
    }
} finally {
    Pop-Location
}

Write-Host ""
Write-Host "Installed adam-lsp + adam-lang extension from staged copy: $stageDir"
Write-Host "To update: uninstall the extension (and 'cargo uninstall adam-lsp' if the binary is locked) before rerunning this task."

# The extension's server-path resolution checks <workspaceRoot>/target/debug before PATH (see
# editors/vscode-adam-lang/src/serverPath.ts), so if this repo's own target/debug/adam-lsp.exe
# ever gets built (e.g. by `cargo build --workspace`), the extension picks that up instead of
# the installed copy — silently defeating the point of installing outside the worktree. Pinning
# adam-lang.serverPath explicitly (highest-priority in that resolution order) avoids that.
$installedBinary = (Get-Command adam-lsp.exe -ErrorAction SilentlyContinue).Source
if (-not $installedBinary) {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
    $installedBinary = Join-Path $cargoHome "bin\adam-lsp.exe"
}
if (-not (Test-Path $installedBinary)) {
    Write-Warning "Could not locate the installed adam-lsp.exe to pin; set the `"adam-lang.serverPath`" VS Code setting manually."
} else {
    $vsCodeSettingsPath = Join-Path $env:APPDATA "Code\User\settings.json"
    if (-not (Test-Path $vsCodeSettingsPath)) {
        Write-Warning "VS Code user settings.json not found at '$vsCodeSettingsPath'; set `"adam-lang.serverPath`" to `"$installedBinary`" manually."
    } else {
        $content = Get-Content -Raw -Path $vsCodeSettingsPath
        $escapedValue = $installedBinary -replace '\\', '\\'
        $newEntry = "`"adam-lang.serverPath`": `"$escapedValue`""
        $existingPattern = [regex]::new('"adam-lang\.serverPath"\s*:\s*"(?:[^"\\]|\\.)*"')
        if ($existingPattern.IsMatch($content)) {
            $newContent = $existingPattern.Replace($content, $newEntry, 1)
        } else {
            $braceIndex = $content.IndexOf('{')
            $newContent = $content.Substring(0, $braceIndex + 1) + "`r`n    $newEntry," + $content.Substring($braceIndex + 1)
        }
        Set-Content -Path $vsCodeSettingsPath -Value $newContent -NoNewline
        Write-Host "Pinned VS Code setting adam-lang.serverPath -> $installedBinary"
    }
}
