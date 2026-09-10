# SAFETY.md

LocalLM's harness is experimental developer tooling. It has had no
independent security audit. Treat it accordingly.

## Posture

- **Local-first is not airtight.** Sessions, settings, and tool records live
  in SQLite on this device, but any external provider, MCP server, plugin, or
  fetched URL can exfiltrate prompt content. Local-first reduces exposure; it
  does not remove it.
- **Least privilege.** Conversations default to Ask for approval. Auto-approve
  reads covers trusted reads only. Full access runs without prompts — use it
  on disposable checkouts, never as a default.
- **Untrusted work gets a boundary.** Local execution runs with your Windows
  account's permissions and is NOT an isolation boundary. Use the Docker
  sandbox provider (no network, capped CPU/memory) or Daytona for untrusted
  repositories. The UI says so wherever execution is configured.
- **Subagents, workflows, memory, and schedules inherit policy.** Children run
  under a reads-only pin unless the parent conversation is Full access (and,
  for schedules, the task explicitly allows writes). Denials become errored
  results, never silent skips.

## What the harness enforces (Rust, not prompts)

Persistence, credentials (vault + Windows Credential Manager), runtime
lifecycle, networking bounds, agent execution, permissions, and timeouts all
live in Rust. The system prompt is not a security boundary. Model-invisible
actions are impossible by construction: anything model-visible is in the
durable log (`docs/session-format-status.md`).

## Telemetry

LocalLM collects no telemetry and ships no analytics. There is nothing to
opt out of; if that ever changes, it will be announced, default-off, and
limited to settings plus project lists.

## Disclosure

Report harness safety issues with reproduction steps and the Trajectory
export of the run. Do not include secrets: keys are redacted from errors,
and secrets in logs or prompts are treated as bugs.
