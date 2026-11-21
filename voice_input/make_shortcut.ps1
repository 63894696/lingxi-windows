# Prisir LingXi desktop shortcut
# ASCII-safe filename avoids GBK/UTF-8 mojibake on Chinese Windows consoles.
# Display name can still show Chinese via Description; shortcut file itself uses Latin.
$ErrorActionPreference = 'Stop'
$python = (Get-Command pythonw -ErrorAction SilentlyContinue).Source
if (-not $python) { $python = (Get-Command python).Source }
$root = Join-Path $env:USERPROFILE 'voice_input'
$script = Join-Path $root 'lingxi_app.py'
$ico = Join-Path $root 'lingxi.ico'
$desktop = [Environment]::GetFolderPath('Desktop')

# Preferred ASCII name (no encoding trap)
$lnkName = 'Prisir LingXi.lnk'
$lnkPath = Join-Path $desktop $lnkName

$ws = New-Object -ComObject WScript.Shell
$sc = $ws.CreateShortcut($lnkPath)
$sc.TargetPath = $python
$sc.Arguments = "`"$script`""
$sc.WorkingDirectory = $root
$sc.IconLocation = "$ico,0"
# Description may contain Chinese; WScript.Shell stores Unicode for .lnk fields
$sc.Description = [string]([char]0x50 + [char]0x72 + [char]0x69 + [char]0x73 + [char]0x69 + [char]0x72 + [char]0x20) + ([char]0x7075) + ([char]0x7280) + ' - voice dictation'
# Simpler: write Description via .NET UTF-16 string literal in file as here-string with BOM-safe chars
$sc.Description = "Prisir LingXi - global voice dictation"
$sc.Save()

Write-Host "shortcut: $lnkPath exists=$(Test-Path -LiteralPath $lnkPath)"

# Remove garbled / legacy Chinese-named launchers that show as 镐电妧 etc.
Get-ChildItem -LiteralPath $desktop -Force | Where-Object {
  $_.Name -like 'Prisir*' -and (
    $_.Extension -eq '.bat' -or
    ($_.Extension -eq '.lnk' -and $_.Name -ne $lnkName)
  )
} | ForEach-Object {
  Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue
  Write-Host "removed legacy: $($_.Name)"
}

# Also clean garbled bat inside voice_input (non-ASCII named start scripts)
Get-ChildItem -LiteralPath $root -Filter '*.bat' -Force | Where-Object {
  $_.Name -ne 'start_lingxi.bat' -and $_.Name -match 'Prisir|lingxi|start'
} | ForEach-Object {
  # keep only ASCII start_lingxi.bat; remove mojibake copies
  if ($_.Name -notmatch '^[A-Za-z0-9_.\-]+\.bat$') {
    Remove-Item -LiteralPath $_.FullName -Force -ErrorAction SilentlyContinue
    Write-Host "removed garbled bat: $($_.Name)"
  }
}
