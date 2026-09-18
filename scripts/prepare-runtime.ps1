param([switch]$IncludeModel, [switch]$Bonsai)
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$localRoot = Join-Path $taskRoot '.local'
$downloadRoot = Join-Path $localRoot 'downloads'
$runtimeRoot = Join-Path $localRoot 'runtime'
if ($Bonsai) { $runtimeRoot = Join-Path $localRoot 'runtime-prism-b9601-68faa14' }
New-Item -ItemType Directory -Force $downloadRoot,$runtimeRoot | Out-Null
$assets = @(
    @{ Name='llama-b10855-bin-win-cuda-12.4-x64.zip'; Hash='4f1e2505e5c3ce0126b2f44c5b87af375960428a4ff98535dfeee8a5fbed8a5b' },
    @{ Name='cudart-llama-bin-win-cuda-12.4-x64.zip'; Hash='8c79a9b226de4b3cacfd1f83d24f962d0773be79f1e7b75c6af4ded7e32ae1d6' }
)
$releaseUrl = 'https://github.com/ggml-org/llama.cpp/releases/download/b10855'
if ($Bonsai) {
    # This model uses legacy group-128 Q2_0. New Prism releases use a different type layout.
    $releaseUrl = 'https://github.com/PrismML-Eng/llama.cpp/releases/download/prism-b9601-68faa14'
    $assets[0] = @{ Name='llama-prism-b1-68faa14-bin-win-cuda-12.4-x64.zip'; Hash='16115de1c186a65d9501dbc38a50f504dc065333abb567cab7c8d3462fb4f42b' }
}
function Get-VerifiedFile([string]$Url, [string]$Destination, [string]$ExpectedHash) {
    if (Test-Path -LiteralPath $Destination) {
        if ((Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash -ieq $ExpectedHash) {
            Write-Output "Verified existing $([IO.Path]::GetFileName($Destination))"
            return
        }
        throw "Existing file hash mismatch: $Destination"
    }
    $partialPath = "$Destination.part"
    & curl.exe --fail --location --retry 3 --continue-at - --output $partialPath $Url
    if ($LASTEXITCODE -ne 0) { throw "Download failed: $Url" }
    if ((Get-FileHash -LiteralPath $partialPath -Algorithm SHA256).Hash -ine $ExpectedHash) {
        throw "Downloaded file hash mismatch: $partialPath"
    }
    Move-Item -LiteralPath $partialPath -Destination $Destination
    Write-Output "Downloaded and verified $([IO.Path]::GetFileName($Destination))"
}
foreach ($asset in $assets) {
    $archive = Join-Path $downloadRoot $asset.Name
    Get-VerifiedFile "$releaseUrl/$($asset.Name)" $archive $asset.Hash
    Expand-Archive -LiteralPath $archive -DestinationPath $runtimeRoot -Force
}
if ($IncludeModel) {
    $modelRoot = Join-Path $localRoot 'models'
    New-Item -ItemType Directory -Force $modelRoot | Out-Null
    if ($Bonsai) {
        Get-VerifiedFile 'https://huggingface.co/prism-ml/Ternary-Bonsai-8B-gguf/resolve/c2aefbeb4b24469cd11579c3384b990404c17a30/Ternary-Bonsai-8B-Q2_0.gguf' (Join-Path $modelRoot 'Ternary-Bonsai-8B-Q2_0.gguf') '3c8d70470a5d97e5a2b9410ddd899cb740116591462626c60cb2fead6448f60b'
    } else {
        Get-VerifiedFile 'https://huggingface.co/prithivMLmods/MiniCPM5-2B-GGUF/resolve/8b969e82c3ea123d604242f6d97a93b98a452070/MiniCPM5-2B.Q6_K.gguf' (Join-Path $modelRoot 'MiniCPM5-2B.Q6_K.gguf') 'd39e78a06dbb9b28ed9a9118b1370e992caf862003c925264ea33789b3e416bc'
    }
}
