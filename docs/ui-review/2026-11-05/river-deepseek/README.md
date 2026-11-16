# River / DeepSeek native review — 5 November 2026

This is a real-provider review of the current embedded LocalLM desktop build. It is an evidence-backed follow-up to `docs/CHAT_CODING_UX_PLAN_2026-11-05.md`, not a completed interface redesign.

## Configuration and execution

- Provider: River, `https://api.river.ai/v1`, OpenAI-compatible Chat Completions.
- Exact model: `deepseek-ai/DeepSeek-V4.1-Flash-1017`, confirmed by the authenticated live model list.
- The key was saved through LocalLM's real Providers UI into its encrypted vault. No plaintext key was added to the repository. The saved provider subsequently passed Test connection and real inference.
- Model limits: 1,048,576 context tokens and 393,216 maximum output tokens, from [River's model documentation](https://docs.river.ai/guides/inference-models/). These are capacity metadata, not a claim that a response used that many tokens. Tool capability remains recorded as unknown; actual file and command calls succeeded in this sample.
- The model was selected with the real Use action and is persisted as LocalLM's preferred model.
- The current frontend and Rust source were built with `npm run tauri -- build --debug --no-bundle`. The frontend build passed, with the existing large-chunk warning; the embedded native build passed.

Native interaction used a private X11 virtual display and an accessibility bus, retaining the actual app-data directory and OS-protected credential store. No localhost browser preview was opened. A local Xvfb package was extracted under ignored `.local/native-tools`; nothing was installed system-wide.

The demo lives under `.local/qa/river-live-2026-11-05/workspace`. The initial request correctly reported disabled tools. After an unreliable native folder-chooser interaction, the app was stopped, its database backed up under the ignored review directory, and the demo workspace path and this conversation's `__workspace` / `__execution` selections were set directly. Approval mode remained **ask**. This configuration step was not a successful end-to-end tool-settings UI acceptance test. Provider/key/model setup did use the real UI.

The successful task read the demo files, edited `sum.js`, created `sum.test.mjs`, ran `node --test sum.test.mjs`, then collected a structured `check_report`: **4 passed, 0 failed, exit 0**, with a complete TAP report. One earlier memory request was denied; only the inspected demo reads, edits and checks were approved. No product source files were edited by the model. `run-provenance.json` records the build fingerprint and sanitized check outcome.

## Captures

All images are native LocalLM captures, not reference-app screenshots or proposed designs.

| Image | Evidence |
| --- | --- |
| `baseline-live-000.png` | Real first-response streaming state; model explains unavailable tools. |
| `baseline-02-approval.png` | Actual modal approval, with session label; unrelated memory request was denied. |
| `baseline-05-execution-approval.png` | Actual requested execution review. |
| `completed-light.png` | Completed footer reports one edited file although two files changed. |
| `diff-expanded-light.png` | Existing shared inline diff, expanded through View changes. |
| `created-file-expanded-light.png` | New-file action expands into raw envelope JSON. |
| `answer-light.png`, `answer-dark.png` | Completed real-provider answer, same saved conversation in both themes. |
| `answer-narrow-light.png`, `answer-narrow-dark.png` | Same conversation at 760×900; not a different mobile implementation. |

Live frame sequences remain under ignored `.local/qa/river-live-2026-11-05/`. Accessible scroll-to was used to reveal selected content; it also shifted ancestor layout in some captures, so header position in those frames is not evidence of ordinary wheel-scroll behavior.

## Findings and the next delivery

### 1. Fix completeness before styling the footer

**Confirmed:** `sum.js` was edited and `sum.test.mjs` created. The footer displays “Edited 1 file” and only `sum.js` (`completed-light.png`). The create result has a path/hash/byte count but no diff. `WorkSummary.tsx` requires a diff, so a successful create is omitted.

**Change:** derive a per-turn change manifest from authoritative tool outcomes. Track create/edit/delete/move and batch patches independently of diff availability. A new file should appear with a Created label and content/artifact review; missing diff must not mean missing file. Distinguish edit churn from net totals and preserve uncertain/rolled-back outcomes. Do not claim that unrelated dirty Git files belong to this turn.

**Acceptance:** this exact replay shows two files, including the new test; repeat edits do not inflate a purported net total; batch patches and failed/unknown outcomes have fixtures. Preserve the existing shared DiffView.

### 2. Give each expansion a useful first view

**Confirmed:** expanding the created-file action opens envelope JSON, including schema, metadata and hash, instead of the created code (`created-file-expanded-light.png`). During live updates, accessible labels briefly included `Edited null`; completed labels later resolved to the actual paths. That transient requires a targeted reproduction before assigning its root cause.

**Change:** use a typed result projection shared by the timeline and footer. File actions show the actual target plus Created/Edited/Deleted, and expand into captured content or a diff. Commands show the command, cwd, measured duration, exit code and bounded tail. Checks show parsed cases/counts and failures. Put authorization provenance, hashes, envelope JSON and schema versions under Technical details. During incomplete argument delivery, show “Preparing edit request” without a guessed filename or `null`.

**Acceptance:** the new-file row opens readable code, check-report opens “4 passed” plus cases, and unsupported tools retain an inspectable fallback. Actual untrusted HTML remains literal. No lifecycle handles are lost when results are bounded.

### 3. Make progress and completion distinct

**Confirmed:** streaming, Stop, approval dialogs, expanded tool rows and final-answer rendering all occurred with the real provider. The final answer includes a process ID, `check_report`, `testsPassed`, `incomplete` and other report fields. These are genuine model output, not fabricated UI evidence, but they make the handoff harder to scan. The check was also run once as a command and again through `check_run` after the verification reminder.

**Change:** establish one compact live status at the current turn; keep commentary quieter than the final answer. Render test evidence in a concise deterministic component. Adjust final-answer guidance to summarize change, result and material limits in ordinary language; retain technical evidence on inspection. Teach the harness to choose the appropriate structured check path initially, without treating arbitrary command exit 0 as proof that tests passed. Do not remove verification to save a round.

**Acceptance:** a normal successful handoff reads “Fixed the empty-array case. Added four tests; all passed,” with actual file/check review nearby. Stop remains reachable. A test that fails, finds no tests, or becomes stale after an edit never receives the same Passed treatment.

### 4. Reclaim space and clarify attention

**Confirmed:** the screenshots show a tall composer area, several navigation/context layers and a Trajectory footer. Trajectory says “0 events” despite durable events and actual tool activity. This is a visibility/recovery defect to investigate; this review did not establish the failing loader or permission boundary. Approval dialogs obscure the transcript even for routine permitted read requests under the selected ask mode.

**Change:** reduce resting composer spacing, retain expandable long prompts and attachments, and make diagnostics opt-in. Keep one understandable workspace/tool-readiness notice with a direct settings action. Preserve explicit approval ownership and review details. Where the user chooses an existing read-approval policy, respect it; do not silently grant Full access. Use in-context attention for approvals, with session routing rather than unrelated modal focus theft.

**Acceptance:** the same desktop fixture gives visibly more transcript space without clipping controls. The narrow composer remains reachable. Journal failures are distinguishable from a genuinely empty trace. Tool readiness explains why a task cannot run before users spend a model round discovering it.

## Reference direction

[OpenAI's Codex app description](https://openai.com/index/introducing-the-codex-app/) documents reviewing changes inside a thread, commenting on diffs and opening them in an editor. That supports using the existing inline review/editor as LocalLM's main completion path. [OpenCode's permission documentation](https://opencode.ai/docs/en/permissions/) documents explicit allow/ask/deny decisions and per-tool/input rules; use it as a behavioral reference for understandable scope and attention, not a reason to broaden LocalLM permissions.

These sources were consulted as documented interaction references. No live Codex/OpenCode instance or usable reference screenshot was inspected during this pass, so this report does not claim a pixel comparison or a matched appearance. The desired composition is quieter activity, useful progressive disclosure, strong answer hierarchy and one trustworthy change/check footer. Keep LocalLM's paper/forest palette, IBM Plex, semantic tokens and shared MarkdownView/CodeBlock/DiffView. Response presentation stays in Settings; no composer mode picker.

## Scope and remaining validation

The configuration and sampled live task are verified. No product redesign or renderer fix was implemented in this pass. No full Rust/frontend regression suite, latency benchmark, 200% zoom review, hidden-session approval sequence, or live reasoning-text disclosure was verified. This sampled successful turn recorded zero reasoning characters; do not manufacture a thinking transcript or timing from its total latency. Narrow/light/dark captures are limited to the shown content, not complete accessibility sign-off.

Implement the first two items together, then re-run the exact demo and compare the captures. They establish accurate, useful review content before spending a pass on spacing and visual polish.
