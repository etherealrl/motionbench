$ErrorActionPreference = 'Stop'
$version = '10.14.1.48'
$name = "tensorrt_cu13_libs-$version-py2.py3-none-win_amd64.whl"
$url = "https://pypi.nvidia.com/tensorrt-cu13-libs/$name"
$expected = '24750993c33de7b31afeb1dc7934ae98a86ef50ffbc240648a06ba4b03504025'
$cache = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../.dependency-cache'))
New-Item -ItemType Directory -Force $cache | Out-Null
$archive = Join-Path $cache $name
if (!(Test-Path -LiteralPath $archive)) {
    & curl.exe --fail --location --retry 2 --silent --show-error --output $archive $url
    if ($LASTEXITCODE -ne 0) { throw 'TensorRT download failed' }
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) {
    throw "TensorRT checksum mismatch; remove $archive and retry"
}
$root = Join-Path $env:USERPROFILE '.motion-bench'
# AppData writes from packaged terminals can be redirected into their private cache.
$destination = Join-Path $root "runtimes/tensorrt/$version"
New-Item -ItemType Directory -Force $destination | Out-Null
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($archive)
try {
    foreach ($entry in $zip.Entries) {
        if ($entry.Name -match '\.dll$' -or $entry.Name -match '^(LICENSE|COPYING|NOTICE)(\.txt)?$') {
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $destination $entry.Name), $true)
        }
    }
} finally { $zip.Dispose() }
foreach ($name in @('nvinfer_10.dll','nvonnxparser_10.dll','nvinfer_plugin_10.dll')) {
    if (!(Test-Path (Join-Path $destination $name))) { throw "TensorRT package is missing $name" }
}
@{ schema = 'motion-bench.tensorrt-runtime.v1'; version = $version; source = $url; sha256 = $expected } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'runtime.json') -Encoding utf8
Write-Output "Installed TensorRT $version (CUDA 13) permanently for this user: $destination"
