param(
    [string]$Archive = "$env:LOCALAPPDATA/motion-bench/cuda13/downloads/cudnn-9.22-cuda13.zip",
    [string]$Log = "$env:LOCALAPPDATA/motion-bench/cudnn-install.log"
)
$ErrorActionPreference = 'Stop'
Start-Transcript -Path $Log -Force
try {
    if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne '11f714ad2699f1d548546fe8797ca563c498d4c66d5a4e9b6c2948325ca8da7d') { throw 'cuDNN archive checksum mismatch' }
    $destination = 'C:\Program Files\NVIDIA\CUDNN\v9.22'
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        foreach ($entry in $zip.Entries) {
            $relative = ($entry.FullName -split '/', 2)[1]
            if (!$relative -or !$entry.Name) { continue }
            $target = [IO.Path]::GetFullPath((Join-Path $destination $relative))
            if (!$target.StartsWith($destination + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid archive path' }
            New-Item -ItemType Directory -Force ([IO.Path]::GetDirectoryName($target)) | Out-Null
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $target, $true)
        }
    } finally { $zip.Dispose() }
    [Environment]::SetEnvironmentVariable('CUDNN_PATH', $destination, 'Machine')
    $bin = Join-Path $destination 'bin/x64'
    $cudaRoot = 'C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3'
    if (Test-Path (Join-Path $cudaRoot 'bin/nvcc.exe')) {
        [Environment]::SetEnvironmentVariable('CUDA_PATH', $cudaRoot, 'Machine')
        [Environment]::SetEnvironmentVariable('CUDA_PATH_V13_3', $cudaRoot, 'Machine')
    }
    $oldPath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $added = @($bin, (Join-Path $cudaRoot 'bin'), (Join-Path $cudaRoot 'bin/x64'))
    $entries = @($oldPath -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -notin $added -and $_ -ine (Join-Path $destination 'bin') })
    [Environment]::SetEnvironmentVariable('Path', (($entries + $added) -join ';'), 'Machine')
    Write-Output "Installed cuDNN 9.22.0.52 (CUDA 13): $destination"
} finally { Stop-Transcript }
