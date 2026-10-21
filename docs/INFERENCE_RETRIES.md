# Inference HTTP retries

The local, OpenAI-compatible, native Claude and subscription inference backends use a shared pre-stream retry layer (`src-tauri/src/inference_retry.rs`). It has no additional dependencies and is not used for tools or provider model discovery.

- At most three harness attempts, with exponential backoff starting at 250 ms and independent 0–125 ms jitter.
- At most eight seconds of scheduled retry waiting. Existing request timeouts still bound each HTTP attempt; the eight seconds is not a new deadline on normal model generation.
- HTTP 408/429 and transient 5xx responses may retry. Known permanent server statuses 501/505/506/508/510/511 do not. Vendor 5xx statuses, including 529, use the same bounded policy.
- Permanent request/auth/quota error codes and known prompt/template/context-validation diagnostics prevent retry even if a runtime labels them 500 or 429.
- Error bodies are inspected with a 16 KiB/one-second bound and retained for existing diagnostics. Oversized or incomplete bodies are marked explicitly. Credentials continue through the existing sanitization boundary.
- `Retry-After` accepts seconds or HTTP dates. A delay beyond the remaining wait budget ends retries instead of sending a request before the server's minimum. Numeric overflow is treated as a long delay. The date parsing references [HTTP date semantics](https://www.rfc-editor.org/rfc/rfc9110.html#section-5.6.7), [Retry-After](https://www.rfc-editor.org/rfc/rfc9110.html#section-10.2.3) and the obsolete-year interpretation in [RFC 5322](https://www.rfc-editor.org/rfc/rfc5322.html#section-4.3).
- Authentication failures, invalid HTTP requests, transport failures and partially received success streams are not retried by this layer. A successful HTTP response is handed to the consumer without reading or replaying its stream.
- Dropping the request future cancels header waits, error-body reads and backoff. Parent and child cancellation continue to use their existing cancellation selects.

Cloned requests preserve URL, authorization, headers and JSON body. Uncloneable request bodies are not replayed. Response extensions carry attempt/wait/stop metadata; final errors report actual retry counts and budget stops. This does not imply that a remote service stops already-started work when the local request is cancelled.

Seven fixture tests cover all four backend protocols, repeated transient errors, bounded attempts/backoff, Retry-After timing/overflow/dates, permanent failures, credential sanitization, cancellation at three wait stages, partial streams and transport failure after a successful HTTP retry. Existing inference protocol tests retain streaming/tool/Unicode/usage behavior and the local chat-template diagnostic regression. SWE-2 Max adversarial review and final full-plan verification remain required.

## Foreground output-limit recovery

A completed stream with a token-limit stop is handled before empty-answer
finalization. The foreground turn gets at most two recovery rounds, charged to
its normal round budget. Offered tools remain available during recovery; a
thinking-only cutoff does not switch the turn into a tool-free final answer.
This is independent of the automatic context-compaction setting.

For local inference, the next round's output reserve grows toward at least 8192
tokens, then doubles, subject to remaining context and the 32768-token local
runtime ceiling. Context is recounted with the actual new
reserve before generation, and tool-result bounding uses that reserve too.
Saved user preferences are unchanged. A progress notice and a durable step-end
event record the recovery.

API inference uses the selected model's maximum output tokens from Providers,
including a manually entered capacity, instead of the local response preference.
Discovered capacities fill missing values without replacing manual values. API
output budgets are not capped at 32768. When output capacity is unknown, OpenAI
Chat Completions, Responses and subscription requests omit the output limit and
use the provider's default; estimated context preflight reserves zero output
tokens in this case, so it cannot guarantee that the eventual output will fit.
Anthropic Messages requires an explicit output limit and asks for that model
capacity to be configured when missing. Context capacity still applies to API
requests. API recovery continues in another response with the same model budget;
it does not claim to increase a budget already at the model maximum.

Unfinished tool batches are discarded before dispatch and are not replayed into
provider history. No sibling call in that truncated batch runs. The model is
asked for a fresh complete call, with small writes followed by patches. Visible
text-only output can continue from its saved response. Exhausted recovery,
permission-denied rounds and already-finalizing rounds fail explicitly on a
token-limit stop; they are not recorded as successful completions.

Socket-free checks cover reserve limits and catalog availability. The two
foreground SSE integration regressions are ignored by default because they
require a permitted host with loopback TCP:

```sh
cd src-tauri
cargo test --lib chat::output_recovery_tests -- --ignored
```

These fixtures exercise an empty cutoff, truncated write arguments followed by
a successful saved file, preserved tool availability, and bounded failure after
repeated cutoffs. Neither fixture launches a browser.
