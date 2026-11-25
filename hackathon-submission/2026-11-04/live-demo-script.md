# LocalLM — three-minute live demo

Record the native Tauri application. Use a small disposable workspace with non-sensitive files, a working Muse Spark 1.3 provider connection, and tools you have already enabled. Keep provider keys off screen. This is a recording plan, not evidence that the workflow has been run.

| Time | Show | Say |
| --- | --- | --- |
| 0:00–0:20 | LocalLM home, then your selected workspace | “AI can explain a task, but getting it done often means switching between a chatbot, files, a terminal, and Git. I built LocalLM to bring that work into one desktop workspace.” |
| 0:20–0:40 | The selected Muse Spark 1.3 model in the app; avoid the API-key screen | “For this project, Meta's Muse Spark 1.3 powers the assistant. LocalLM gives it selected project context and tools, while I control what it can access and execute.” |
| 0:40–1:10 | Submit the first prompt below; show real file reads and the response | “Here I ask the assistant to inspect a small project and explain one function. It reads the workspace through enabled tools, so the answer can be grounded in the actual code.” |
| 1:10–1:40 | Request one small change; show the approval request and approve the intended edit | “Now I ask for a small improvement. LocalLM shows the proposed tool action before it runs. The approval policy is enforced by the application, and the action stays visible in the conversation.” |
| 1:40–2:10 | Review the resulting diff, then run a suitable existing check | “I can review the resulting change and run a check from the same workspace. The tool output tells me what actually happened. If a check fails, the failure stays visible.” |
| 2:10–2:35 | Editor, terminal, saved conversation activity; optionally one working connector | “The editor, terminal, Git review, and saved tool activity are part of the same environment. MCP connectors and reusable skills extend the workspace when the task needs them.” |
| 2:35–2:50 | Models & runtime settings, without changing the working configuration | “LocalLM also supports compatible local GGUF models through llama.cpp. Muse Spark is the hosted model used here; local inference is a separate choice.” |
| 2:50–3:00 | Return to the reviewed result; show repository URL | “LocalLM connects Meta's model intelligence to practical work, with visible actions and user control. The source is available on GitHub. Thank you.” |

## Suggested prompts

1. “Read this project's README and inspect one small function. Explain what it does and suggest one small improvement. Do not edit anything yet.”
2. “Implement that one improvement. Keep the change small and explain the resulting diff.”
3. “Run the existing check relevant to this change. Report the actual outcome, including failures or unavailable checks.”

Choose the specific function and check before recording. Do not run the demo against valuable uncommitted work. Avoid extra setup during the recording. If inference or checks take too long, cut waiting time and mark the cut; preserve the real result. Never replace failed output with an invented success.

## Video title

LocalLM — Muse Spark 1.3 in a Desktop AI Workspace | Meta AI Developer Hackathon

## Video description

LocalLM connects an AI assistant to workspace files, code execution, MCP tools, and Git review, with permission controls and saved tool activity. This submission uses Meta's Muse Spark 1.3 as the hosted assistant model. Compatible local GGUF inference is also supported separately through llama.cpp.

Source: https://github.com/Lawliet2004/localLM
