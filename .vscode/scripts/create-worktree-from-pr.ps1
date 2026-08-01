$prsJson = gh pr list --state open --json number,title,headRefName --limit 100
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$prs = @($prsJson | ConvertFrom-Json)

if ($prs.Count -eq 0) {
    Write-Host "No open pull requests found."
    exit 0
}

Write-Host ""
Write-Host "Open pull requests:"
for ($i = 0; $i -lt $prs.Count; $i++) {
    $pr = $prs[$i]
    Write-Host ("  [{0}] #{1} {2} ({3})" -f $i, $pr.number, $pr.title, $pr.headRefName)
}
Write-Host ""

$selection = Read-Host "Enter number to create worktree from (or press Enter to cancel)"
if ([string]::IsNullOrWhiteSpace($selection)) {
    Write-Host "Cancelled."
    exit 0
}

$index = 0
if (-not [int]::TryParse($selection, [ref]$index) -or $index -lt 0 -or $index -ge $prs.Count) {
    Write-Error "Invalid selection: $selection"
    exit 1
}

$pr = $prs[$index]
$branch = "pr-$($pr.number)"
$worktreePath = ".claude/worktrees/$branch"

if (Test-Path $worktreePath) {
    Write-Error "Worktree path '$worktreePath' already exists."
    exit 1
}

git fetch origin "pull/$($pr.number)/head:$branch"
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

git worktree add $worktreePath $branch
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

tokensave init $worktreePath
tokensave branch add $branch --path $worktreePath
code --new-window $worktreePath
