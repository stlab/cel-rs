$paths = git worktree list --porcelain | Where-Object { $_ -match '^worktree ' } | ForEach-Object { $_ -replace '^worktree ', '' }
$worktrees = @($paths | Where-Object { ($_ -replace '\\', '/') -match '\.claude/worktrees/' })

if ($worktrees.Count -eq 0) {
    Write-Host "No worktrees found under .claude/worktrees/"
    exit 0
}

Write-Host ""
Write-Host "Worktrees available for removal:"
for ($i = 0; $i -lt $worktrees.Count; $i++) {
    Write-Host ("  [{0}] {1}" -f $i, $worktrees[$i])
}
Write-Host ""

$selection = Read-Host "Enter number to remove (or press Enter to cancel)"
if ([string]::IsNullOrWhiteSpace($selection)) {
    Write-Host "Cancelled."
    exit 0
}

$index = 0
if (-not [int]::TryParse($selection, [ref]$index) -or $index -lt 0 -or $index -ge $worktrees.Count) {
    Write-Error "Invalid selection: $selection"
    exit 1
}

$target = $worktrees[$index]
git worktree remove "$target"
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
tokensave branch gc
