# Build the experimental ZAYA implementation separately from the normal runtime.
# Requires Visual Studio 2022 C++ Build Tools with its CMake component.
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$localRoot = Join-Path $taskRoot '.local'
$revision = '3750f9ce7ac20f7a905b43d9f20ad1050884f6c7'
$sourceRoot = Join-Path $localRoot "llama.cpp-$revision"
$archive = Join-Path $localRoot 'downloads/llama-zaya-3750f9c.tar.gz'
$archiveHash = '4450a6b172e53001e6b16708912e547d78b637d4095b5f8c6b592ce3ae17781d'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
if (!(Test-Path -LiteralPath $vswhere)) { throw 'Install Visual Studio 2022 C++ Build Tools with CMake first.' }
$vsRoot = & $vswhere -latest -version '[17.0,18.0)' -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$vsRoot) { throw 'Visual Studio 2022 C++ Build Tools were not found.' }
$cmake = Join-Path $vsRoot 'Common7/IDE/CommonExtensions/Microsoft/CMake/CMake/bin/cmake.exe'
if (!(Test-Path -LiteralPath $cmake)) { throw 'Add the C++ CMake tools component to Visual Studio Build Tools.' }
New-Item -ItemType Directory -Force (Split-Path $archive) | Out-Null
if (!(Test-Path -LiteralPath $archive)) {
    & curl.exe --fail --location --retry 2 --output "$archive.part" "https://codeload.github.com/Juste-Leo2/llama.cpp/tar.gz/$revision"
    if ($LASTEXITCODE -ne 0) { throw 'ZAYA runtime source download failed.' }
    Move-Item -LiteralPath "$archive.part" -Destination $archive
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ine $archiveHash) {
    throw "ZAYA source archive hash mismatch: $archive"
}
if (!(Test-Path -LiteralPath (Join-Path $sourceRoot 'CMakeLists.txt'))) {
    & tar.exe -xzf $archive -C $localRoot
    if ($LASTEXITCODE -ne 0) { throw 'ZAYA runtime source extraction failed.' }
}
$buildRoot = Join-Path $sourceRoot 'build'
& $cmake -S $sourceRoot -B $buildRoot -G 'Visual Studio 17 2022' -A x64 `
    -DGGML_CUDA=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF `
    -DLLAMA_BUILD_APP=OFF -DLLAMA_BUILD_UI=OFF -DLLAMA_USE_PREBUILT_UI=OFF -DLLAMA_OPENSSL=OFF
if ($LASTEXITCODE -ne 0) { throw 'ZAYA runtime configuration failed.' }
# Two compiler processes keep peak RAM use modest on this laptop.
& $cmake --build $buildRoot --config Release --target llama-server --parallel 2
if ($LASTEXITCODE -ne 0) { throw 'ZAYA runtime compilation failed.' }
$runtime = Join-Path $buildRoot 'bin/Release/llama-server.exe'
& $runtime --version
if ($LASTEXITCODE -ne 0) { throw 'Built ZAYA runtime could not start.' }
Write-Output "ZAYA CPU runtime: $runtime"
Write-Output 'Verify with: node scripts/zaya-runtime-smoke.mjs'
