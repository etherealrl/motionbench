param([switch]$InstallSystem)
$ErrorActionPreference = 'Stop'
$cacheRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../.dependency-cache'))
New-Item -ItemType Directory -Force $cacheRoot | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Download($name, $url, $hash) {
    $path = Join-Path $cacheRoot $name
    if (!(Test-Path -LiteralPath $path)) {
        & curl.exe --location --fail --retry 2 --silent --show-error --output $path $url
        if ($LASTEXITCODE -ne 0) { throw "Download failed: $name" }
    }
    if ($hash -and (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $hash) { throw "Checksum mismatch: $name" }
    return $path
}
function Extract-Runtime($archive, $destination, $pattern, $noticePrefix) {
    New-Item -ItemType Directory -Force $destination | Out-Null
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        foreach ($entry in $zip.Entries) {
            $notice = $entry.Name -match '^(LICENSE|LICENSE-CODE|ThirdPartyNotices)(\.txt)?$'
            if (($entry.FullName -match $pattern) -or $notice) {
                $name = if ($notice) { $noticePrefix + '-' + ($entry.Name -replace '^LICENSE$', 'LICENSE.txt') } else { $entry.Name }
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $destination $name), $true)
            }
        }
    } finally { $zip.Dispose() }
}

$runtime = Join-Path $PSScriptRoot '../../src-tauri/runtime'
# the hash is pinned like the others, since asking GitHub for it hits its rate limit on shared CI runners
$archive = Download 'onnxruntime-1.27.1-cuda13.zip' 'https://github.com/microsoft/onnxruntime/releases/download/v1.27.1/onnxruntime-win-x64-gpu_cuda13-1.27.1.zip' '1a88828fb1ca78c9920637acf0f8d8ce40a854bef874128c55193e3d1a2b9ef8'
Extract-Runtime $archive $runtime '/lib/(onnxruntime|onnxruntime_providers_(shared|cuda|tensorrt))\.dll$' 'ort-cuda'
$archive = Download 'ort-directml.nupkg' 'https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime.directml/1.24.4/microsoft.ml.onnxruntime.directml.1.24.4.nupkg' '57e9f11b73437bef7a309496135d4c1f96b1a8e9ddba60013fa27bfc1d788681'
Extract-Runtime $archive (Join-Path $runtime 'directml') '^runtimes/win-x64/native/.*\.dll$' 'ort-directml'
$archive = Download 'directml.nupkg' 'https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg' '4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9'
Extract-Runtime $archive (Join-Path $runtime 'directml') '^bin/x64-win/DirectML\.dll$' 'directml'

$installedRuntime = Join-Path $env:USERPROFILE '.motion-bench/runtimes/onnx'
New-Item -ItemType Directory -Force $installedRuntime | Out-Null
Copy-Item -Path (Join-Path $runtime '*.dll'), (Join-Path $runtime '*.txt') -Destination $installedRuntime -Force
New-Item -ItemType Directory -Force (Join-Path $installedRuntime 'directml') | Out-Null
Copy-Item -Path (Join-Path $runtime 'directml/*.dll'), (Join-Path $runtime 'directml/*.txt') -Destination (Join-Path $installedRuntime 'directml') -Force
Write-Output "Installed ONNX Runtime providers permanently for this user: $installedRuntime"

if ($InstallSystem) {
    $installer = Download 'cuda_13.3.1_windows.exe' 'https://developer.download.nvidia.com/compute/cuda/13.3.1/local_installers/cuda_13.3.1_windows.exe' 'd68839fcce644576f0a1c6b066e0c5bc146a62db2fcac9fc6b4e6418e7ec533f'
    $process = Start-Process -FilePath $installer -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ArgumentList @('-s','cudart_13.3','cublas_13.3','cublas_dev_13.3','cufft_13.3','cufft_dev_13.3','nvrtc_13.3','nvrtc_dev_13.3','nvcc_13.3','crt_13.3','nvvm_13.3','nvjitlink_13.3')
    if ($process.ExitCode -ne 0) { throw "CUDA installation failed: $($process.ExitCode)" }
    $archive = Download 'cudnn-9.22-cuda13.zip' 'https://developer.download.nvidia.com/compute/cudnn/redist/cudnn/windows-x86_64/cudnn-windows-x86_64-9.22.0.52_cuda13-archive.zip' '11f714ad2699f1d548546fe8797ca563c498d4c66d5a4e9b6c2948325ca8da7d'
    $script = Join-Path $PSScriptRoot 'install-cudnn.ps1'
    $process = Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -Wait -PassThru -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File', ('"' + $script + '"'), '-Archive', ('"' + $archive + '"'))
    if ($process.ExitCode -ne 0) { throw "cuDNN installation failed: $($process.ExitCode)" }
}
Write-Output 'ONNX Runtime CUDA and DirectML packages are ready. CUDA uses the permanent system installation.'
