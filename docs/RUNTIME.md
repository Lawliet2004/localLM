# Runtime provenance

Development runtime: ggml-org/llama.cpp release b10855, Windows x64 CUDA 12.4. Archives and SHA-256 digests are pinned in scripts/prepare-runtime.ps1 using GitHub release asset metadata.

Model: prithivMLmods/MiniCPM5-2B-GGUF revision 8b969e82c3ea123d604242f6d97a93b98a452070, MiniCPM5-2B.Q6_K.gguf. Size 2,070,227,904 bytes. SHA-256 d39e78a06dbb9b28ed9a9118b1370e992caf862003c925264ea33789b3e416bc, obtained from Hugging Face LFS metadata. This is a community quantization of OpenBMB's model.

`powershell -ExecutionPolicy Bypass -File scripts/prepare-runtime.ps1 -IncludeModel` downloads resumably and verifies artifacts before extraction/use. Assets stay in ignored .local/. Runtime validation is not yet complete.
