# LocalLM — product

A Windows desktop app (Tauri + React) for running a local GGUF model through llama.cpp, or a tested OpenAI-compatible loopback provider, as a tool-using agent over a project folder.

## Users and scene
Developers and power users running small models on their own GPU, often for long agent tasks (research, coding, benchmarking). They sit in the app for hours, usually in a dim room next to an editor or terminal.

## Surfaces
- **Chat (Operate):** one conversation per task, grouped under project folders. The final answer is the point; the agent's work stays available but quiet.
- **Settings (Operate):** a full-window screen with a grouped rail on the left. It covers General, Appearance, Keyboard shortcuts, Models, Runtime, Generation, Model files, Diagnostics, Tools, Connectors, Skills, Execution, Web search, Browser, Documents, and Data & archive.
- **Inspector:** a right-hand panel with review, terminal, browser, and files.

## Truths the UI must keep
- Model inference and conversation storage are local. Web retrieval, downloads, and configured connectors can use external services. Permissions are explicit: Ask, Auto-approve reads, and Full access.
- Each chat keeps its own tools and permission mode. The last choice becomes the default for new chats.
- Local code is unsandboxed by default. An enforced mode confines spawned commands to the workspace in a Windows AppContainer with no network unless enabled. Docker execution is deferred; cloud (Daytona) execution cannot be started. Historical records remain readable.
- Document collections are local-only: folders are indexed into SQLite and searched on-device, and the assistant cites file paths with line numbers. Nothing in a collection is uploaded.
- Saved recipes can run a prompt on demand or on an interval; scheduled runs need enforced isolation so unattended work cannot write outside the sandbox.
