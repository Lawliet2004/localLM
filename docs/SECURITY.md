# Trust boundaries

The UI is trusted application code. Model text, Markdown links, tool descriptions/results, skill files and OAuth metadata are untrusted data. None of these grant process, filesystem or connector permissions.

Assets: conversation history, workspace files, service credentials, GPU/CPU resources and third-party account actions.

Controls implemented so far: parameterized SQLite operations; restrictive production CSP; Markdown without raw HTML or remote image loading; native IPC validation for runtime/generation settings; random model API key; loopback runtime; removal of inherited LLAMA_* runtime options; bounded model output and stream events; cancellable generation; child cleanup; verified downloads. Further validation remains required before release.

Connector design: Rust MCP SDK handles protocol negotiation and sessions. HTTPS required for remote endpoints; explicit localhost HTTP permitted for local tools. Credentials are encrypted in app-data files using AES-256-GCM with connector-specific authenticated data. The random encryption key is protected through Windows Credential Manager. Credentials are never returned by read APIs or included in prompts. OAuth uses the SDK's PKCE/state validation and a bounded local callback listener. Registry/tool annotations are advisory; model-issued tools require a user-visible approval unless a scoped policy explicitly permits them. Remote results are size-limited and cannot trigger local execution by themselves.

Abuse cases to test: script/HTML in model output; malformed SSE; credentials in endpoint URLs; mismatched OAuth state; expired/revoked credentials; redirects leaking headers; unbounded tool catalogs/results; model selecting unavailable tools; cancellation while awaiting approval; hostile skill archives/symlinks; workspace traversal; process escape and orphan subprocesses. These are release requirements, not claims of tests already completed.


Workspace file operations use cap-std directory capabilities. Only the explicitly selected root is opened with ambient authority; relative operations stay within that capability. Native tests exercise Windows junction escape attempts for both reading and creating files. Absolute paths, parent traversal, alternate data streams, device names, malformed arguments, binary text and oversized reads are rejected. Creation uses create_new so existing content is preserved. Edits stage and sync a sibling file, check the read hash again, and replace through the pinned directory capability; failed staging or replacement preserves the original. This filesystem boundary does not sandbox local process execution.

Local execution defaults to unsandboxed. Approval shows the exact code, language and timeout. Interpreter paths are user-configurable absolute executable paths. Processes start in the selected workspace; in unsandboxed mode they retain the Windows account's filesystem and network permissions. Unrelated inherited environment variables are removed; this does not prevent code from accessing the account's files or credential stores. Windows Job Objects provide lifecycle cleanup; in unsandboxed mode they are not a permission boundary. process-wrap 10.0.0 is used after tests exposed lost cleanup flags in 9.1.0; the unused MCP child-process feature was removed until local stdio transport can use a verified lifecycle implementation.

## Enforced execution isolation (AppContainer)

Settings → Execution can switch local execution to enforced mode. Every spawned command — local code runs, the agent terminal, verification checks, and stdio connectors — then runs inside a Windows AppContainer named per workspace:

- **Filesystem:** the container SID gets full access only on the workspace root; ancestor folders get traverse + read-attributes rights that never propagate to other folders. Interpreters' own directories and user-listed toolchain directories get read+execute. Reads, writes and junction escapes outside the root are denied by the OS.
- **Working directory:** AppContainer forbids changing the process working directory, so it is set at creation. PowerShell is placed on an `LL:` drive rooted at the workspace because its provider cannot follow the inherited location.
- **Network:** with network off the token gets no capabilities — TCP, UDP and loopback all fail. Enabling network grants `internetClient`; LAN additionally grants `privateNetworkClientServer`. Loopback stays blocked in every mode (AppContainer restriction).
- **Processes and handles:** children inherit the container token; `bInheritHandles` is off and only the three stdio handles are passed. A Job Object caps the container at 32 processes and 2 GiB of commit, applies UI restrictions, and kills the tree when the run ends.
- **In-process tools** cannot be covered by the OS boundary; under enforced mode with network off the app blocks browser, web search, web fetch and cloud tools itself and reports "Network blocked by execution policy". Built-in file tools stay inside the workspace via cap-std as before.
- **Failure policy:** if the profile, ACL grant or process launch fails, the command is refused — never run unsandboxed. Leaving enforced mode revokes recorded ACL grants and deletes the sandbox profiles.
- **Limitations:** kernel exploits are out of scope — this is a software boundary, not a VM. LPAC is optional and breaks interpreters that read HKLM keys. Isolation covers spawned processes only, not memory, registry or UI capture outside the listed limits.

## Conversation permission modes

The user explicitly requested Ask, automatic approval and Full access modes. The native agent now reads a persisted conversation permission enum once at the start of a turn. Ask remains the default, including migration of older records. Auto-approve permits only known built-in workspace reads/listings based on the tool backend and name. Full access authorizes every selected tool without prompts; selection validation, argument validation, capability-backed workspace boundaries, execution process cleanup, cancellation, timeout and output limits remain active. Full access local execution is unsandboxed and can access the account's files/network. The UI displays the mode even when the tool picker is collapsed, and each audit records authorization provenance.
