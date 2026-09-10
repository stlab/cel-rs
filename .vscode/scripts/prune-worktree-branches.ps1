$porcelain = git worktree list --porcelain
$activeBranches = @($porcelain | Where-Object { $_ -match '^branch refs/heads/(.+)$' } | ForEach-Object { $matches[1] })

$allWorktreeBranches = @(git branch --list 'worktree-*' | ForEach-Object { $_ -replace '^\*?\s*', '' })
$orphaned = @($allWorktreeBranches | Where-Object { $activeBranches -notcontains $_ })

if ($orphaned.Count -eq 0) {
    Write-Host "No orphaned worktree-* branches found."
    exit 0
}

$merged = @(git branch --merged main --list 'worktree-*' | ForEach-Object { $_ -replace '^\*?\s*', '' })

Write-Host ""
Write-Host "Orphaned local worktree-* branches (no active worktree):"
foreach ($branch in $orphaned) {
    $status = if ($merged -contains $branch) { "merged into main" } else { "NOT merged into main" }
    Write-Host ("  {0}  [{1}]" -f $branch, $status)
}
Write-Host ""

$confirm = Read-Host "Delete all $($orphaned.Count) branches listed above? (y/N)"
if ($confirm -ne 'y' -and $confirm -ne 'Y') {
    Write-Host "Cancelled."
    exit 0
}

foreach ($branch in $orphaned) {
    git branch -D "$branch"
}
tokensave branch gc
