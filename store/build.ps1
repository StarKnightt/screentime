# Build the Microsoft Store package (MSIX) for Screentime for Windows.
#
#   powershell -File store\build.ps1                       unsigned .msix for Partner Center
#   powershell -File store\build.ps1 -TestCert <thumbprint> also a test-signed copy for a local install
#
# Output: store\out\Screentime_1.0.1.0_x64.msix (unsigned; Microsoft signs it on submission)
#         store\out\test\Screentime_1.0.1.0_x64.test.msix (only with -TestCert)
# Needs the Windows 10/11 SDK (MakeAppx, MakePri, SignTool) and Rust.
param([string]$TestCert = "")
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $PSScriptRoot "out"
# Run a native tool and fail on its exit code only: cargo writes progress to stderr,
# which Windows PowerShell would otherwise turn into a terminating error when redirected.
function Run([string]$what) { $ErrorActionPreference = "Continue"; & $args[0] $args[1..($args.Count - 1)]; if ($LASTEXITCODE) { throw "$what failed" } }
$stage = Join-Path $out "stage"

$sdk = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
  Sort-Object { [version]$_.Directory.Parent.Name } | Select-Object -Last 1
if (-not $sdk) { throw "Windows SDK not found. Install it with: winget install Microsoft.WindowsSDK.10.0.26100" }
$bin = $sdk.DirectoryName
"SDK tools: $bin"

Push-Location $root
try {
  Run "cargo build" cargo build --release --locked -p screentime-app -p screentime-cli
  Run "assets tool" rustc -O --edition 2021 store\tools\assets.rs -o target\store-assets.exe

  if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
  New-Item -ItemType Directory -Force $stage | Out-Null
  Copy-Item target\release\screentime-app.exe (Join-Path $stage "Screentime.exe")
  Copy-Item target\release\screentime-tracker.exe (Join-Path $stage "screentime-tracker.exe")
  Copy-Item LICENSE (Join-Path $stage "LICENSE.txt")
  Copy-Item store\AppxManifest.xml (Join-Path $stage "AppxManifest.xml")
  & target\store-assets.exe (Join-Path $stage "Assets")
  if ($LASTEXITCODE) { throw "assets failed" }
  # The generator writes plain (uncompressed) PNGs and MakeAppx stores PNGs as they
  # are, so recompress them with Windows' own encoder (alpha is kept).
  Add-Type -AssemblyName System.Drawing
  Get-ChildItem (Join-Path $stage "Assets") -Filter *.png | ForEach-Object {
    $bmp = New-Object System.Drawing.Bitmap $_.FullName
    $tmp = "$($_.FullName).tmp"
    $bmp.Save($tmp, [System.Drawing.Imaging.ImageFormat]::Png); $bmp.Dispose()
    Move-Item -Force $tmp $_.FullName
  }

  # resources.pri maps Assets\X.png to its scale and targetsize variants.
  $pricfg = Join-Path $out "priconfig.xml"
  & "$bin\makepri.exe" createconfig /cf $pricfg /dq en-US /pv 10.0.0 /o | Out-Null
  # One resources.pri with every scale: split resource packs only work in a bundle.
  $cfg = [xml](Get-Content $pricfg)
  $pk = $cfg.resources.packaging
  if ($pk) { [void]$pk.ParentNode.RemoveChild($pk) }
  $cfg.Save($pricfg)
  & "$bin\makepri.exe" new /pr $stage /cf $pricfg /mn (Join-Path $stage "AppxManifest.xml") /of (Join-Path $stage "resources.pri") /o | Out-Null
  if ($LASTEXITCODE) { throw "makepri failed" }

  $version = ([xml](Get-Content store\AppxManifest.xml)).Package.Identity.Version
  $msix = Join-Path $out "Screentime_${version}_x64.msix"
  & "$bin\makeappx.exe" pack /o /h SHA256 /d $stage /p $msix | Out-Null
  if ($LASTEXITCODE) { throw "makeappx failed" }
  "unsigned package: $msix"
  "sha256: $((Get-FileHash $msix).Hash.ToLower())"

  if ($TestCert) {
    $test = Join-Path $out "test"
    New-Item -ItemType Directory -Force $test | Out-Null
    $signed = Join-Path $test "Screentime_${version}_x64.test.msix"
    Copy-Item $msix $signed -Force
    & "$bin\signtool.exe" sign /fd SHA256 /sha1 $TestCert /s My $signed | Out-Null
    if ($LASTEXITCODE) { throw "signtool failed" }
    "test-signed package: $signed"
  }
} finally {
  Pop-Location
}
