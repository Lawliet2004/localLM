# Model-visible tool results

Tool dispatch uses `locallm.tool-envelope/1` across parent, PTC and child model loops. Individual backend helpers may return their native shape internally; their model-facing callers normalize it before storing, bounding and replaying the result.

The stable fields are `schemaVersion`, `status`, `data`, `error`, `artifacts` and `metadata`. Status is `success`, `partial`, `failure`, `cancelled` or `unknown`. Failed outcomes carry a code, message, retryability and side-effect certainty. Older execution envelopes gain explicit certainty at the shared boundary. Missing evidence that a write had no effect becomes `unknown`, with automatic replay disabled.

Original backend data remains in `data`. Legacy fields needed by existing consumers remain beside the envelope when they do not conflict with envelope keys. For example, a native transaction's `committed` or `rolled_back` state is available under `data.status`; the outer `status` describes the requested tool call. A successful requested rollback is a successful recovery call, while a failed patch that rolled back is a failure. Connector business status fields remain ordinary data.

Malformed or unsupported declarations in the `locallm.tool-envelope/` namespace produce explicit failure envelopes. Metadata must be an object and artifacts an array; malformed metadata cannot panic during enrichment. Native pre-dispatch refusal markers remain distinct from connector domain fields. Generic connector errors cannot claim safe write replay simply by returning `denied`, `paused` or a matching error code.

Bounded results retain their error/uncertainty metadata and artifact pointers. A truncated success becomes `partial`; truncating an error never turns it into a success. Continuation reads typed outcome fields with compatibility fallback for historical data. Unconfirmed writes are not automatically replayed.

Research lifecycle tools use the same user run controls and durable ledger. A model pause cannot take ownership of a pending user pause, and a model resume cannot release it or revive a terminal run. Lock and persistence failures are reported. The parent's completion checkpoint merges the durable lifecycle state so its earlier cached copy cannot undo a pause or cancellation.

Independent regressions cover malformed/unsupported envelopes, ambiguous connector writes, domain status collisions, actual patch recovery, cleanup errors, poisoned control locks, rejected SQLite writes, user-pause priority, cached-ledger completion and an actual failed subprocess. These supplement the backend-family dispatch, artifact and continuation tests. Full-goal acceptance remains in the implementation plan.
