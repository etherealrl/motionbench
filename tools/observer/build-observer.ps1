param([string]$WdkVersion = '10.0.26100.6584', [string]$HardwareId, [switch]$TestSign)
$ErrorActionPreference = 'Stop'
if ($TestSign -and !$HardwareId) { throw 'Test signing requires an exact mouse hardware ID' }
$projectRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$cache = Join-Path $projectRoot ".dependency-cache/wdk-$WdkVersion"
$kit = Join-Path $cache 'package/c'
if (!(Test-Path -LiteralPath $kit)) {
    New-Item -ItemType Directory -Force $cache | Out-Null
    $archive = Join-Path $cache 'wdk.zip'
    & curl.exe --fail --location --silent --show-error --output $archive "https://api.nuget.org/v3-flatcontainer/microsoft.windows.wdk.x64/$WdkVersion/microsoft.windows.wdk.x64.$WdkVersion.nupkg"
    if ($LASTEXITCODE -ne 0) { throw 'WDK package download failed' }
    Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $cache 'package')
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -prerelease -products '*' -property installationPath
if (!$vs) { throw 'MSVC build tools not found' }
$compilerRoot = Get-ChildItem (Join-Path $vs 'VC/Tools/MSVC') -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'bin/Hostx64/x64/cl.exe') } | Sort-Object Name | Select-Object -Last 1
$compiler = Join-Path $compilerRoot.FullName 'bin/Hostx64/x64/cl.exe'
$linker = Join-Path $compilerRoot.FullName 'bin/Hostx64/x64/link.exe'
$sdk = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10'
$sdkVersion = '10.0.26100.0'
$out = Join-Path $projectRoot 'target/observer'
New-Item -ItemType Directory -Force $out | Out-Null
$includes = @(
    (Join-Path $kit "Include/$sdkVersion/km"), (Join-Path $kit "Include/$sdkVersion/shared"),
    (Join-Path $kit 'Include/wdf/kmdf/1.15'), (Join-Path $sdk "Include/$sdkVersion/shared"),
    (Join-Path $sdk "Include/$sdkVersion/ucrt"), (Join-Path $compilerRoot.FullName 'include')
) | ForEach-Object { '/I' + $_ }
$source = Join-Path $projectRoot 'drivers/mouse-observer/observer.c'
$object = Join-Path $out 'observer.obj'
& $compiler /nologo /c /kernel /W4 /WX /O2 /GS /Gz /Zl /TC /D_AMD64_ /DAMD64 /D_WIN64 /DWINNT /DNTDDI_VERSION=0x0A000000 /D_WIN32_WINNT=0x0A00 @includes "/Fo$object" $source
if ($LASTEXITCODE -ne 0) { throw 'Observer compilation failed' }
$libs = @(('/LIBPATH:' + (Join-Path $kit "Lib/$sdkVersion/km/x64")), ('/LIBPATH:' + (Join-Path $kit 'Lib/wdf/kmdf/x64/1.15')))
& $linker /nologo /NODEFAULTLIB /DRIVER /SUBSYSTEM:NATIVE /ENTRY:FxDriverEntry /MACHINE:X64 /DYNAMICBASE /NXCOMPAT /INTEGRITYCHECK "/OUT:$(Join-Path $out 'MotionBenchObserver.sys')" $object @libs ntoskrnl.lib hal.lib BufferOverflowFastFailK.lib WdfDriverEntry.lib WdfLdr.lib
if ($LASTEXITCODE -ne 0) { throw 'Observer linking failed' }
Copy-Item -LiteralPath (Join-Path $projectRoot 'drivers/mouse-observer/MotionBenchObserver.inf') -Destination $out
$catalog = Join-Path $out 'MotionBenchObserver.cat'
if (Test-Path -LiteralPath $catalog) { Remove-Item -LiteralPath $catalog }
if ($HardwareId) {
    if ($HardwareId -notmatch '^HID\\VID_[0-9A-F]{4}&PID_[0-9A-F]{4}(?:&[A-Z0-9_]+)*$') { throw 'Supply an exact mouse HID hardware ID with VID/PID and no wildcards' }
    $inf = Join-Path $out 'MotionBenchObserver.inf'
    $text = [IO.File]::ReadAllText($inf).Replace('HID\MOTION_BENCH_SELECT_EXACT_MOUSE_HARDWARE_ID', $HardwareId)
    [IO.File]::WriteAllText($inf, $text, [Text.Encoding]::ASCII)
}
& (Join-Path $kit "tools/$sdkVersion/x64/infverif.exe") /w (Join-Path $out 'MotionBenchObserver.inf')
if ($LASTEXITCODE -ne 0) { throw 'Observer INF validation failed' }
if ($TestSign) {
    $subject = 'CN=Motion Bench Local Driver Test'
    $certificate = Get-ChildItem Cert:\CurrentUser\My | Where-Object {
        $_.Subject -eq $subject -and $_.Issuer -eq $subject -and $_.HasPrivateKey -and
        $_.NotAfter -gt (Get-Date).AddDays(7) -and
        ($_.EnhancedKeyUsageList.ObjectId -contains '1.3.6.1.5.5.7.3.3')
    } | Sort-Object NotAfter -Descending | Select-Object -First 1
    if (!$certificate) {
        $certificate = New-SelfSignedCertificate -Type CodeSigningCert -Subject $subject `
            -FriendlyName 'Motion Bench local driver testing' -CertStoreLocation Cert:\CurrentUser\My `
            -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 -KeyExportPolicy NonExportable `
            -NotAfter (Get-Date).AddYears(1)
    }
    $publicCertificate = Join-Path $out 'MotionBenchObserver.cer'
    Export-Certificate -Cert $certificate -FilePath $publicCertificate -Force | Out-Null
    foreach ($store in @('Cert:\CurrentUser\Root', 'Cert:\CurrentUser\TrustedPublisher')) {
        if (!(Test-Path (Join-Path $store $certificate.Thumbprint))) {
            Import-Certificate -FilePath $publicCertificate -CertStoreLocation $store | Out-Null
        }
    }
    $signTool = Join-Path $sdk "bin/$sdkVersion/x64/signtool.exe"
    & $signTool sign /fd SHA256 /s My /sha1 $certificate.Thumbprint (Join-Path $out 'MotionBenchObserver.sys')
    if ($LASTEXITCODE -ne 0) { throw 'Observer binary test signing failed' }
}
if ($HardwareId) {
    & (Join-Path $kit "bin/$sdkVersion/x86/Inf2Cat.exe") "/driver:$out" /os:10_VB_X64,10_CO_X64,10_GE_X64
    if ($LASTEXITCODE -ne 0) { throw 'Observer catalog generation failed' }
}
if ($TestSign) {
    & $signTool sign /fd SHA256 /s My /sha1 $certificate.Thumbprint $catalog
    if ($LASTEXITCODE -ne 0) { throw 'Observer catalog test signing failed' }
    foreach ($file in @((Join-Path $out 'MotionBenchObserver.sys'), $catalog)) {
        & $signTool verify /pa /v $file
        if ($LASTEXITCODE -ne 0) { throw "Signature verification failed: $file" }
    }
    & $signTool verify /pa /c $catalog (Join-Path $out 'MotionBenchObserver.sys')
    if ($LASTEXITCODE -ne 0) { throw 'Catalog does not cover the signed driver' }
    & $signTool verify /pa /c $catalog (Join-Path $out 'MotionBenchObserver.inf')
    if ($LASTEXITCODE -ne 0) { throw 'Catalog does not cover the device-specific INF' }
}
@{
    schema = 'motion-bench.observer-build.v1'; wdk = $WdkVersion
    wdk_sha256 = (Get-FileHash -LiteralPath (Join-Path $cache 'wdk.zip') -Algorithm SHA256).Hash
    compiler = $compiler; hardware_id = $HardwareId; signed = [bool]$TestSign
    signing = $(if ($TestSign) { 'local_test_certificate' } else { 'unsigned' })
    certificate_thumbprint = $(if ($TestSign) { $certificate.Thumbprint } else { $null })
    certificate_expires = $(if ($TestSign) { $certificate.NotAfter.ToUniversalTime().ToString('o') } else { $null })
    driver_sha256 = (Get-FileHash -LiteralPath (Join-Path $out 'MotionBenchObserver.sys') -Algorithm SHA256).Hash
    inf_sha256 = (Get-FileHash -LiteralPath (Join-Path $out 'MotionBenchObserver.inf') -Algorithm SHA256).Hash
    catalog_sha256 = $(if (Test-Path $catalog) { (Get-FileHash -LiteralPath $catalog -Algorithm SHA256).Hash } else { $null })
    revision = (& git -C $projectRoot rev-parse HEAD)
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $out 'build.json') -Encoding utf8
Get-FileHash -LiteralPath (Join-Path $cache 'wdk.zip') -Algorithm SHA256
Write-Output "Built $(Join-Path $out 'MotionBenchObserver.sys'). Signing: $(if ($TestSign) { 'local test certificate' } else { 'unsigned' }). Not installed."
