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
if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    Write-Error "LOCALAPPDATA is not set; cannot pick a staging directory outside the worktree."
    exit 1
}

$repoRoot = (Get-Location).Path
$stageDir = Join-Path $env:LOCALAPPDATA "cel-rs\adam-lsp-install"

New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
Write-Host "Staging workspace copy at: $stageDir"

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
        if (-not (Test-Path "node_modules")) {
            npm install
            if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        }

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
