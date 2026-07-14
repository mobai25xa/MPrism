#Requires -Version 7
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "== MPrism production audit ==" -ForegroundColor Cyan

$failures = @()

function Assert-True($cond, $msg) {
  if (-not $cond) { $script:failures += $msg; Write-Host "FAIL: $msg" -ForegroundColor Red }
  else { Write-Host "OK:   $msg" -ForegroundColor Green }
}

$tauriPath = Join-Path $root 'apps/desktop/src-tauri/tauri.conf.json'
$capPath = Join-Path $root 'apps/desktop/src-tauri/capabilities/default.json'
$tauri = Get-Content $tauriPath -Raw | ConvertFrom-Json
$cap = Get-Content $capPath -Raw | ConvertFrom-Json

Assert-True ($tauri.version -eq '0.1.0') 'tauri version is 0.1.0'
Assert-True ($tauri.app.windows[0].width -eq 1280 -and $tauri.app.windows[0].height -eq 800) 'default window 1280x800'
Assert-True ($tauri.app.windows[0].minWidth -eq 900 -and $tauri.app.windows[0].minHeight -eq 600) 'min window 900x600'
Assert-True (@($tauri.bundle.targets) -contains 'nsis') 'NSIS target enabled'
Assert-True (@($tauri.bundle.targets).Count -eq 1) 'only NSIS target'
Assert-True ($tauri.bundle.createUpdaterArtifacts -eq $false) 'updater artifacts disabled'
Assert-True ($tauri.bundle.windows.webviewInstallMode.type -eq 'downloadBootstrapper') 'WebView2 bootstrapper configured'

$csp = [string]$tauri.app.security.csp
Assert-True ($csp -match "default-src 'self'") "CSP default-src self"
Assert-True ($csp -notmatch 'unsafe-eval') 'CSP has no unsafe-eval'
Assert-True ($csp -notmatch "connect-src [^;]*\*") 'CSP connect-src is not wildcard host'
Assert-True ($csp -match "frame-ancestors 'none'") 'CSP frame-ancestors none'

$permIds = @()
foreach ($p in $cap.permissions) {
  if ($p -is [string]) { $permIds += $p }
  elseif ($null -ne $p.identifier) { $permIds += [string]$p.identifier }
}
Assert-True ($permIds -contains 'core:default') 'capabilities include core:default'
Assert-True ($permIds -contains 'opener:allow-open-url') 'capabilities include scoped opener'
$bad = $permIds | Where-Object {
  $_ -like 'shell:*' -or $_ -like 'fs:*' -or $_ -like 'http:*' -or $_ -like 'updater:*' -or $_ -eq 'opener:default'
}
Assert-True (@($bad).Count -eq 0) 'no shell/fs/http/updater permissions'
Assert-True ($permIds -notcontains 'opener:default') 'opener:default not granted (scoped only)'

$scanRoots = @(
  'apps/desktop/src',
  'apps/desktop/src-tauri/src',
  'apps/desktop/src-tauri/tests',
  'crates',
  'docs'
)
$secretHits = @()
foreach ($rel in $scanRoots) {
  $dir = Join-Path $root $rel
  if (-not (Test-Path $dir)) { continue }
  Get-ChildItem $dir -Recurse -File -Include *.rs,*.ts,*.tsx,*.json,*.md |
    Where-Object { $_.FullName -notmatch '\\gen\\' } |
    ForEach-Object {
      $lines = Select-String -Path $_.FullName -Pattern 'sk-[A-Za-z0-9]{16,}|api[_-]?key\s*[:=]\s*["''][^"'']+["'']' -CaseSensitive:$false -ErrorAction SilentlyContinue
      foreach ($hit in $lines) {
        if ($hit.Line -match 'sk-secret-test-key|api_key_present|api_key_input|type: "keep"|type: "clear"|type: "replace"|redact|脱敏') {
          continue
        }
        $secretHits += "$($hit.Path):$($hit.LineNumber):$($hit.Line.Trim())"
      }
    }
}
Assert-True ($secretHits.Count -eq 0) 'no unexpected plaintext key-like secrets in source'
if ($secretHits.Count -gt 0) { $secretHits | Select-Object -First 20 | ForEach-Object { Write-Host $_ } }

if ($failures.Count -gt 0) {
  Write-Host "`nAudit failed with $($failures.Count) issue(s)." -ForegroundColor Red
  exit 1
}
Write-Host "`nAudit passed." -ForegroundColor Green
exit 0
