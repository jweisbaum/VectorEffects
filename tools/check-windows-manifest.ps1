# Fails unless the given executable carries the application manifest that
# selects Common Controls 6. `crates/ve-app/build.rs` embeds it through the
# linker rather than through tauri-build's resource, so that test programs
# get it too; an application built without it dies before `main` with
# STATUS_ENTRYPOINT_NOT_FOUND, and nothing else in the build would notice.
param([Parameter(Mandatory = $true)][string]$Exe)

if (-not (Test-Path $Exe)) {
  Write-Output "::error title=Windows manifest::$Exe was not built"
  exit 1
}
$text = [System.Text.Encoding]::ASCII.GetString([System.IO.File]::ReadAllBytes($Exe))
# The attribute as a manifest spells it; the bare assembly name could be
# anywhere, this cannot.
if (-not $text.Contains('name="Microsoft.Windows.Common-Controls"')) {
  Write-Output "::error title=Windows manifest::$Exe has no Common Controls 6 manifest and would not start"
  exit 1
}
Write-Output "$Exe carries the application manifest."
