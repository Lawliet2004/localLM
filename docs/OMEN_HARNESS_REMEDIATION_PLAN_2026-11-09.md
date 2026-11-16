# LocalLM remediation plan from the Omen run

The Omen goal-file turn reached the model round limit with a timed-out release build, missing acceptance evidence, and a partial handoff. This plan covers the shared LocalLM paths exercised by that turn and the defects reproduced during the follow-up audit.

| Stage | Change | Acceptance |
| --- | --- | --- |
| 1. Workspace and process safety | Bind snapshots and staged reverts to their original workspace identity and reviewed target state. Stop an attached shell process when its wait is cancelled, and retain typed process diagnostics. | A revert staged in A cannot touch B or overwrite changes made after staging. Cancelling an attached command prevents a later write. An explicitly returned process can still be polled or stopped. |
| 2. Durable turn state | Bind resume to the pending request and original task. Settle reply, run, task, and journal together. Classify operations by their outcome and effects independently of output excerpting. Scope Plan mode instructions to approved plans. | Discard cannot affect the next ordinary request. Startup can distinguish interrupted work from delivered results. A successful clipped edit remains successful. Partial runs publish their stop reason and pending work. |
| 3. Goal and verification | Resolve an explicit goal file through the selected workspace. Store its identity, acceptance items, and check evidence across bounded segments. Use the effective task budget consistently. | The Omen-style fixture preserves its goal and independent gates after timeout or pause. Unrelated commands cannot satisfy a required check. Resuming continues pending work without replaying settled writes. |
| 4. Tool and prompt efficiency | Keep a compact, stable coding tool set with discovery and batching. Remove duplicate model-visible content, return exact page cursors, version prompt definitions, and size response reserves per round. | Stable requests retain prefix reuse. The model can read the whole goal efficiently, gets actionable validation errors, and can finalize on providers with and without tool-choice control. |
| 5. Chat and diagnostics | Surface partial outcomes, pending checks, resumable tasks, and process state. Report progress and final-answer latency separately, and reduce long-history reconciliation work. | Saved and live results render consistently. Budget exhaustion exposes a usable resume action. The latest activity is available without replaying the whole journal first. |
| 6. Regression and outcome validation | Exercise the actual backend and React paths with disposable workspaces and scripted inference. Run the existing suites and keep the measurement scope explicit. | Safety, recovery, typed evidence, and a representative patch-to-verification handoff have regression coverage. Record test counts and distinguish fixture results from live provider, native WebView, and elapsed unattended operation. |

## Running a goal

Select the project and enable Workspace files and Local execution. Send `/goal GOAL.md` with a workspace-relative path. Goal files can define milestones with numbered level-two headings, checkboxes, commands in a `Verification commands` section, and handoff items in `Definition of done`. Commands in the verification section are independent gates executed from the workspace root.

The chat records a partial handoff when work reaches the model round or time budget, or when acceptance items still need evidence. Use **Resume task** on that handoff to resume its specific checkpoint. LocalLM checks the workspace identity and goal-file hash on resume and checks recorded source hashes before carrying forward named gate evidence.

Managed shell processes and checks can use a lifetime of up to 60 minutes under **Settings → Bash → Timeout (seconds)**. A running check can be observed for up to 30 seconds per report call. The parent model has a 30-minute active segment and can hand off a running command while the application is open.

## Validation scope

The regression fixtures use disposable workspaces. They cover cross-workspace undo, files changed after review, attached shell cancellation, exact resume binding, a rollback during turn intake, recovery of an older torn terminal record, goal-file scope, and independent source-bound gates. The React tests cover pending-work rendering, exact Resume dispatch, live shell handles, and recent activity paging.

| Check | Result |
| --- | --- |
| Rust library tests | 1,116 passed, 37 ignored |
| Rust library check | Passed |
| Vitest | 912 passed, 1 skipped |
| TypeScript check and Vite production build | Passed. Vite reported large JavaScript chunks. |
| Whitespace check | Passed |

Pending live validation includes a River DeepSeek rerun with the Omen goal, cache telemetry from that rerun, and a native desktop smoke test. Continuing work after LocalLM exits requires reopening the application and resuming the saved task. A separate unattended executor would need its own execution and approval policy.
