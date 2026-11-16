# Inference and tool-result failures — 6 November 2026

The saved failures come from interrupted model generations, alongside a reproducible application bug that loses artifact-page content. They do not show a general failure to accept valid tool calls. Four failures contain an upstream `inference_timeout` error; the screenshot's failure lost its underlying HTTP error details before persistence.

## Evidence from saved runs

Inspected the live application database in read-only mode. It contains 11 runs: five completed, five failed, and one cancelled. All five failures used River's `deepseek-ai/DeepSeek-V4.1-Flash-1017` through Chat Completions.

Times below are IST (Asia/Kolkata), on 6 November 2026. Round numbers are zero-based. Duration is the failed generation round, rather than the whole task.

| Run started | Failed round | Duration | Persisted error |
| --- | --- | --- | --- |
| 13:37:37 | 4 | 124.993 s | Upstream `inference_timeout`, `server_error` |
| 13:45:37 | 0 | 127.188 s | Upstream `inference_timeout`, `server_error` |
| 13:48:41 | 0 | 126.357 s | Upstream `inference_timeout`, `server_error` |
| 14:56:08 | 0 | 125.286 s | Upstream `inference_timeout`, `server_error` |
| 18:09:53 | 4 | 100.859 s | `error decoding response body` — screenshot |

The four explicit upstream errors include `The inference request timed out. Please retry.` They establish that the provider's response reported an inference failure. They do not establish which internal serving component timed out or exclude request characteristics as a contributor. The roughly 125-second pattern is an observation, not a documented River deadline.

In the screenshot's run, the first four rounds completed. Five tools were accepted and returned: `project_inspect`, `workspace_list_files`, `development_guide`, and two `artifact_read` calls. The next generation produced some text and then failed. No write tool was dispatched in that run.

Across the database's 83 tool audit rows, 80 allowed calls returned success or bounded partial output. Two allowed reads failed because the files were not UTF-8 text. One memory call was explicitly denied by the user. None of the five failed generations persisted a tool-schema or tool-argument rejection as its terminal error.

## Application defects reproduced and patched

### Artifact paging loses the information it is supposed to retrieve

The frontend guide was captured as a 13,005-byte artifact. The model requested a page from that artifact; normalization duplicated the page in the canonical `data` field and compatibility fields. Bounding then filled the result budget before adding the outer envelope. When the wrapper exceeded the budget, it degraded to a pointer to a newly captured artifact, dropping the page text and its original cursor.

The actual screenshot run produced successive 24,247-byte and 28,881-byte page artifacts. Both model-visible results lacked the requested excerpt and pointed to another artifact. The model followed the new pointer once, then proceeded without retrieving the useful guide text.

`artifacts.rs` now fits a successful page's text and cursor within the complete serialized result, including envelope overhead and JSON/UTF-8 expansion. It preserves the original artifact ID, advances by the number of characters actually delivered, and avoids capturing another artifact for an existing page. The normalized regression failed before the fix. It now reconstructs the entire original text at 512-, 2,048-, and 8,192-byte budgets, with and without compatibility fields.

### Clipped output is displayed as failed execution

`Chat.tsx` treated envelope status `partial` as a failed tool, even when `error` was null. Artifact bounding deliberately uses `partial` for successful output that was clipped. The screenshot run contained three such bounded results.

The failure classification now follows actual errors, denials, uncertain outcomes, and nonzero exit codes. A clipped result without an error no longer gets a failed badge. A regression reproduced the false badge before the fix; a separate error-bearing case still reports failure.

### HTTP body failures lose their cause

Foreground chat and child generation converted `reqwest::Error` directly with `to_string()`. For a body-decoding error, that prints only `error decoding response body`; it omits the source chain. This is separate from LocalLM's JSON and tool-argument parsers, which have different messages.

Both paths now use a shared backend formatter that distinguishes read timeouts, preserves a bounded source-chain explanation, removes the request URL, and redacts the configured credential. Local HTTP fixtures exercise truncated successful bodies and stalled bodies, and verify that failures do not trigger another request. The exact cause of the historical screenshot's disconnect cannot be recovered from the flattened saved string.

### A shorter HTTP deadline overrides the chat loop's longer waits

Remote clients previously had a 180-second total request deadline, while chat allowed up to 600 seconds for the initial response and 300 seconds between chunks. Reqwest's total deadline includes reading the response body, so an active long generation could be cut off at three minutes despite continued output.

The remote client deadline is now 600 seconds, matching the existing chat request budget and local client policy. This was a separate reliability defect: the recorded failures at approximately 101 and 125 seconds did not hit the old 180-second deadline. See the [reqwest timeout documentation](https://docs.rs/reqwest/latest/reqwest/struct.ClientBuilder.html#method.timeout) for the distinction between a total deadline and a read timeout.

## Recovery and remaining uncertainty

`inference_retry.rs` retries selected HTTP error statuses before consuming a successful stream. It deliberately does not retry partially received successful streams. Chat stops when those streams fail; “Retry last prompt” submits the original user prompt as a new message, rather than resuming the failed generation round.

The patch preserves that replay protection. It does not introduce automatic retries of writes, commands, or incomplete tool calls. A separate recovery improvement would need to restart only an unfinished model round from the last completed tool checkpoint, retain completed outcomes, discard unfinished arguments, and bound any retry attempts. River also [documents that incomplete streams cannot be resumed and that completed tool actions must not be replayed](https://docs.river.ai/guides/inference-limits/).

The selected model is configured with a 393,216-token output capacity, which existing application policy uses as the per-request output limit. This permits long generations, but does not prove why the upstream requests timed out. No model limit or saved provider setting was changed.

The changes repair local loss of tool output, misleading status reporting, the shorter HTTP deadline, and insufficient error detail. They do not establish or repair River's internal inference failures. No paid live inference was issued during this investigation, and no browser preview was used.

## Validation

Targeted verification passed: 22 artifact tests, 8 inference retry/transport tests, 13 SSE tests, 14 tool-call assembly tests, and 48 chat component tests. Both the normalized paging test and the clipped-output badge test failed before their fixes.

- `cargo test --lib`: 1,037 passed, 0 failed, 34 ignored. Ignored tests are not claimed as passing. The non-ignored local inference HTTP fixtures ran successfully on this host.
- `cargo check --lib`: passed.
- `npm run typecheck`: passed.
- `npx vitest run`: 891 passed, 1 failed, 1 skipped. The failure is `src/App.test.tsx` → `independent running conversations` → `renders journal-backed reasoning received on the live channel before a journal poll`; its immediate `Thinking…` assertion also fails when run alone. This live-reasoning integration issue remains unresolved; the suite is not fully green.
- `git diff --check`: passed.

## Follow-up review: remaining backend issues

These findings describe the source after the initial patches above. No additional backend code was changed during this review.

1. **Advertised file-write sizes exceed the model-call transport limit.** `workspace_patch.rs` advertises up to 1,048,576 characters for `write_file` content, but `ToolCalls::push` and `validated` reject an entire arguments JSON string above 65,536 bytes. A disposable Rust harness imported the actual `tool_calls.rs` source: 65,500 ASCII content bytes plus the path/JSON overhead passed at 65,536 serialized bytes; 65,536 content bytes and 70,000 content bytes both failed with `Tool call exceeds its size limit.` This is a contract mismatch, separate from provider failures. Resolve per-tool input budgets and document/support incremental writes; do not execute incomplete arguments or indiscriminately remove size limits. Source: `src-tauri/src/tool_calls.rs:154`, `src-tauri/src/workspace_patch.rs:1838`.

2. **Bounding a large file read can drop the file identity needed for an edit.** The generic result-bounding path preserves neither `path` nor the file's `sha256`; it introduces `_sha256` for the serialized tool-result artifact instead. The saved partial `file_read` result has no original file hash in either canonical data or compatibility fields. Existing small reads retain it. The full hash can be recovered through another read/artifact request, but ordinary read-edit workflows should retain it alongside the excerpt, with an unambiguous distinction from the artifact checksum. Source: `src-tauri/src/artifacts.rs:786`, `src-tauri/src/workspace.rs:725`.

3. **Response completion automatically changes every pending todo to completed.** `send_message_inner` performs this whenever its result is `Ok(true)`, without checking that those individual tasks succeeded. This also affects bounded final answers that explicitly describe partial work. Ending generation must not certify unfinished task execution. Keep pending/in-progress tasks, or record an explicit partial/unverified task outcome. Source: `src-tauri/src/chat.rs:4658`.

4. **Artifact persistence errors are discarded.** After bounding a result, foreground dispatch ignores `save_artifact` errors and can then send the generated artifact pointer to the model. If artifact storage fails while smaller message writes still succeed, that pointer cannot retrieve the promised full result. Propagate or explicitly represent capture failure without replaying the original operation. This error-handling defect is visible in the source; no such storage failure was observed in the saved runs. Source: `src-tauri/src/chat.rs:3391`.

5. **Failed-generation recovery still stops at the turn boundary.** A stream/body failure aborts the turn; the existing resumable checkpoint UI applies to paused research tasks, while Retry last prompt submits another user message. A safer continuation should retain completed tool outcomes and restart only the unfinished generation with a strict retry budget. This remains an improvement to implement, not an existing automatic retry guarantee.

Verification is currently advisory: an unchecked write receives one reminder, after which another final answer is accepted. The existing `verify_before_finish_nudges_once_after_an_unchecked_write` test deliberately exercises this behavior. An explicit distinction between delivered, verified, partial, and unverified outcomes would improve accuracy without forcing inappropriate checks or bypassing disabled tools.

### Where the measured time goes

Saved app preparation times were approximately 0.11–0.35 seconds. In the screenshot run, preparation took 0.241 seconds and tool execution totaled approximately 0.01 seconds; the first model token arrived at 6.516 seconds, and the final model round failed after 100.859 seconds.

One completed run took 261.7 seconds with 64 main model rounds plus one presentation round. Tool execution totaled 13.0 seconds and approval waiting was zero. Its audit contains 51 `run_code` calls. This establishes substantial orchestration/model-round latency in that example, rather than slow file I/O. It does not establish that every round was avoidable. Existing batching and independent-read concurrency should be used where dependencies and permission policy allow it; reducing rereads caused by lost metadata should also reduce requests.

The backend can reduce redundant round trips and preserve reliable execution state. These changes cannot alone guarantee the provider's generation speed or the model's correctness. Measure preparation, first model token, first visible answer, tool time, approval time, and round count separately.
