$porcelain = git worktree list --porcelain
$entries = @()
$currentPath = $null
foreach ($line in $porcelain) {
    if ($line -match '^worktree (.+)$') {
        $currentPath = $matches[1]
    } elseif ($line -match '^branch refs/heads/(.+)$' -and $currentPath) {
        $entries += [pscustomobject]@{ Path = $currentPath; Branch = $matches[1] }
        $currentPath = $null
    }
}
$worktrees = @($entries | Where-Object { ($_.Path -replace '\\', '/') -match '\.claude/worktrees/' })

if ($worktrees.Count -eq 0) {
    Write-Host "No worktrees found under .claude/worktrees/"
    exit 0
}

Write-Host ""
Write-Host "Worktrees available for removal:"
for ($i = 0; $i -lt $worktrees.Count; $i++) {
    Write-Host ("  [{0}] {1}  (branch: {2})" -f $i, $worktrees[$i].Path, $worktrees[$i].Branch)
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
git worktree remove "$($target.Path)"
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
git branch -D "$($target.Branch)"
tokensave branch gc
