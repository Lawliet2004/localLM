# Trust boundaries

The UI is trusted application code. Model text, Markdown links, tool descriptions/results, skill files and OAuth metadata are untrusted data. None of these grant process, filesystem or connector permissions.

Assets: conversation history, workspace files, service credentials, GPU/CPU resources and third-party account actions.

Controls implemented so far: parameterized SQLite operations; restrictive production CSP; Markdown without raw HTML or remote image loading; native IPC validation for runtime/generation settings; random model API key; loopback runtime; removal of inherited LLAMA_* runtime options; bounded model output and stream events; cancellable generation; child cleanup; verified downloads. Further validation remains required before release.

Connector design: Rust MCP SDK handles protocol negotiation and sessions. HTTPS required for remote endpoints; explicit localhost HTTP permitted for local tools. Credentials are encrypted in app-data files using AES-256-GCM with connector-specific authenticated data. The random encryption key is protected through Windows Credential Manager. Credentials are never returned by read APIs or included in prompts. OAuth uses the SDK's PKCE/state validation and a bounded local callback listener. Registry/tool annotations are advisory; model-issued tools require a user-visible approval unless a scoped policy explicitly permits them. Remote results are size-limited and cannot trigger local execution by themselves.

Abuse cases to test: script/HTML in model output; malformed SSE; credentials in endpoint URLs; mismatched OAuth state; expired/revoked credentials; redirects leaking headers; unbounded tool catalogs/results; model selecting unavailable tools; cancellation while awaiting approval; hostile skill archives/symlinks; workspace traversal; process escape and orphan subprocesses. These are release requirements, not claims of tests already completed.

