# LocalLM — hackathon submission

Copy the text under each field into the form. Meta integration wording reflects your confirmation that Muse Spark 1.3 powers the assistant inside LocalLM. The other feature claims are grounded in the current repository; this preparation did not execute a fresh app acceptance test.

## Project name

LocalLM

## Tagline / short description

A desktop AI workspace that connects Meta's Muse Spark 1.3 to your files, code, and tools—with permissions and review built in.

## Project Description

LocalLM turns an AI conversation into a practical workspace for getting things done. Developers and makers often have to copy information between a chatbot, an editor, a terminal, and external services. That breaks context and makes it harder to see what an AI actually changed.

LocalLM brings those workflows into one desktop application. Powered by Meta's Muse Spark 1.3 for this submission, the assistant can work with selected workspace files and enabled tools, helping users understand a project, make changes, and inspect the results. An integrated editor, terminal, Git review, MCP connectors, and reusable skills connect the conversation to real work.

The user controls the assistant's access through approval modes and permission rules. Tool decisions and results are recorded, file changes can be reviewed, and larger outputs are saved as inspectable artifacts.

Built with Rust, Tauri 2, React, and TypeScript, LocalLM also supports compatible local GGUF models through llama.cpp. Users can choose local inference or a configured hosted provider. Muse Spark 1.3 runs through the hosted provider path; local inference is a separate option. The goal is an AI workspace that combines useful model capabilities with visible actions and user control.

## Public GitHub Repository

https://github.com/Lawliet2004/localLM

## 3-Minute Demo Video

Upload `locallm-overview.mp4` to a video host, then paste its shareable URL here. A local file path is not a submission URL. The generated video is a narrated overview with explanatory diagrams, not a recording of the application. For an actual product demonstration, record the workflow in `live-demo-script.md` and use that video URL.

## Describe your use of Meta products to build the submitted project

LocalLM uses Meta's Muse Spark 1.3 as the model powering the in-app assistant for this submission. The model provides the reasoning and responses, while LocalLM supplies the workspace context and the tools available for the user's task.

LocalLM's agent harness connects the assistant to selected file operations, code execution, and MCP tools. It validates tool inputs and enforces permissions before dispatch, then returns tool results to the conversation so the assistant can continue working. The desktop interface lets users inspect tool activity, generated artifacts, and file changes.

Muse Spark 1.3 is used through LocalLM's configured hosted-provider connection. The application's separate llama.cpp integration supports compatible local GGUF models. This lets the same workspace support both hosted model workflows and local inference without claiming that Muse Spark 1.3 runs on the user's device.

## Why are Meta's models important to your solution?

LocalLM needs a model that can do more than answer a single question: it must interpret a user's goal, work with project context, choose appropriate tools, and use the results to continue a task. Muse Spark 1.3 provides the model intelligence behind that experience in this submission.

Meta describes Muse Spark 1.3 as improving agentic and coding tasks, including longer workflows, tool use, and instruction following. Those capabilities align closely with LocalLM's purpose: connecting a conversation to practical work across files, code, and external tools.

Our contribution is the environment around the model. LocalLM handles workspace integration, permission checks, tool execution, persistent history, and review. Together, the model and the desktop harness let users move from a request to an inspectable result while retaining control over the actions taken.

## Optional Most Viral track — draft X post

Meet LocalLM: a desktop AI workspace powered by Meta's Muse Spark 1.3. Bring files, code, tools, and Git review into one conversation—with permission controls and inspectable results. Built for the Meta AI Developer Hackathon.

https://github.com/Lawliet2004/localLM

Publish this yourself if entering the track, then paste the actual post URL into the form. No post has been published by this preparation.

## Sources and submission notes

- Current feature descriptions: `README.md`, `docs/PERMISSIONS.md`, and `docs/WORKSPACE.md` in this checkout.
- Public repository visibility verified at https://github.com/Lawliet2004/localLM . Public visibility alone does not establish open-source licensing; check the repository's license against the hackathon requirements.
- Official model capabilities: https://research.meta.ai/blog/introducing-muse-spark-1-3 . Capability claims above are a paraphrase of Meta's announcement, not independent LocalLM benchmark results.
- Muse Spark 1.3 use inside LocalLM was confirmed by you; no provider credentials or private conversations were inspected.
- Only fields visible in the supplied screenshots are covered, plus an optional name, tagline, and X-post draft. No form was submitted and no video was uploaded.
