Describe "create-worktree-from-pr.ps1" {
    It "creates a worktree whose branch tracks the PR head" {
        $scriptPath = Join-Path $PSScriptRoot "create-worktree-from-pr.ps1"
        $remotePath = Join-Path $TestDrive "remote.git"
        $repositoryPath = Join-Path $TestDrive "repository"
        $shimPath = Join-Path $TestDrive "bin"
        $worktreePath = Join-Path $repositoryPath ".claude/worktrees/pr-235"
        New-Item -ItemType Directory -Path $shimPath | Out-Null
        New-Item -ItemType Directory -Path (Split-Path $worktreePath) | Out-Null

        git init --bare $remotePath | Out-Null
        git init $repositoryPath | Out-Null
        git -C $repositoryPath config user.email "test@example.com"
        git -C $repositoryPath config user.name "Test User"
        Set-Content (Join-Path $repositoryPath "file.txt") "main"
        git -C $repositoryPath add file.txt
        git -C $repositoryPath commit -m "Initial commit" | Out-Null
        git -C $repositoryPath branch -M main
        git -C $repositoryPath remote add origin $remotePath
        git -C $repositoryPath push -u origin main | Out-Null
        git -C $repositoryPath switch -c feature | Out-Null
        Set-Content (Join-Path $repositoryPath "file.txt") "feature"
        git -C $repositoryPath commit -am "Feature commit" | Out-Null
        git -C $repositoryPath push -u origin feature | Out-Null
        git -C $repositoryPath switch main | Out-Null

        @'
@echo off
if "%1 %2"=="pr list" (
  echo [{"number":235,"title":"Test PR","headRefName":"feature"}]
  exit /b 0
)
if "%1 %2"=="pr checkout" (
  git worktree add --track -b %5 %7 origin/feature
  exit /b %ERRORLEVEL%
)
exit /b 1
'@ | Set-Content (Join-Path $shimPath "gh.cmd")
        "@exit /b 0" | Set-Content (Join-Path $shimPath "tokensave.cmd")
        "@exit /b 0" | Set-Content (Join-Path $shimPath "code.cmd")

        $oldPath = $env:PATH
        $oldLocation = Get-Location
        try {
            $env:PATH = "$shimPath;$oldPath"
            Set-Location $repositoryPath

            & $scriptPath -Selection "0"

            git -C $worktreePath rev-parse --abbrev-ref --symbolic-full-name "@{upstream}" |
                Should Be "origin/feature"
        }
        finally {
            Set-Location $oldLocation
            $env:PATH = $oldPath
            if (Test-Path $worktreePath) {
                git -C $repositoryPath worktree remove --force $worktreePath 2>$null
            }
            $global:LASTEXITCODE = 0
        }
    }
}
