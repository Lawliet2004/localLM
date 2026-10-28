# Conversation activity contract

This is the current specification for the conversation transcript. It supersedes
older answer-first panels, category-colored activity cards, and dotted terminal
trees described in historical design notes. App navigation and Settings retain
their existing appearance. Response presentation remains Settings-only.

## Presentation

`src/activity.css` owns the transcript styles and loads last. Semantic activity
and transcript colors live in `src/tokens.css`. Light conversations use the saved
surface; dark conversations use the neutral charcoal transcript palette.

Commentary, exposed reasoning, tool operations and the final answer follow the
recorded sequence. Prose uses the saved chat font and chat size plus two pixels,
with 1.72 line height. Adjacent feed entries have 16px separation (12px in narrow
layouts). Activity labels share that font size; command/file targets use the
saved chat size and metadata uses two pixels less, with a 12px floor. Rows use
18px icons, consistent gaps and 30px minimum height (24px in compact density).

Rows are neutral, borderless disclosures. Expanded exploration, command details
and file diffs use indentation without decorative tree rails. Consequential
operations, pending approval, failures, interruptions and running work remain
separate and inspectable. Only adjacent successful file exploration may fold.
Errors/deletions and additions/success retain their semantic status colors.
Keep visible keyboard focus and respect reduced motion. No synthetic reasoning
text or invented durations: models without exposed reasoning use the existing
live activity status. Thought duration appears only from recorded measurements.

## File changes

One rounded bordered bar follows the answer, collapsed initially. Its label
opens the file list; View changes opens recorded diffs using the shared DiffLines
renderer. There is no unsupported Undo action. Read actual transaction changes[]
as well as legacy single-file results. Restored artifact results must belong to
the current conversation. Failed, rolled-back and uncertain mutations never
become successful file summaries. A created or binary file without a diff still
appears, with unavailable line totals explained instead of fabricated +0/−0.
Totals describe recorded edit operations, not a net repository diff.

## Event and completion truth

Journal sequence determines chronology; missing journal tool rows fall back to
recorded timestamps. Join by run and call identity, never by array position.
Partial journals retain tools, and disclosure identities survive incoming rounds.
The verification_status journal records launch/pending, failure, command-only and
passing test evidence separately. Starting a process is not passing verification;
a later workspace mutation invalidates earlier check observations.

Model-step failures preserve existing tool results and name the failed round.
Repeated identical invalid calls are bounded; uncertain actions are not replayed
automatically. Completion of the assistant turn does not guarantee every external
service or command succeeded.

## Acceptance

Unit/integration checks cover real result envelopes, replay, interleaving,
artifacts, failures and controls. They do not establish rendered fidelity.
Representative native captures in light/dark and supported sizes remain the
visual acceptance requirement. Localhost browser preview remains prohibited;
no alternate preview route may be used to bypass that restriction.

## Live round and outcome reconciliation

Transient model rounds retain run/step identity and overlay their own journal
step until committed rows arrive. The channel sequence counter is not the
journal counter. Live output buffers and operation rows also include the
conversation/run/step/call tuple, while persisted message IDs remain unchanged.
One shared outcome interpretation drives activity folding, status labels and
successful file-change eligibility. Explicit partial, cancelled and unknown
outcomes stay visible. See CODING_AGENT_WORKFLOW_2026-10-25.md for the scoped
runtime and native-rendering evidence.
