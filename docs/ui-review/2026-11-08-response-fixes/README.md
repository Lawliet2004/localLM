# Implemented response UI — 8 November 2026

These are captures of the real embedded Tauri application with an isolated synthetic transcript. They are not design mockups or screenshots of another product. No paid model, real account or browser preview was used. Existing user data was not opened or modified.

The light review uses 1280×840; the dark review uses the supported 760×560 minimum. Source/binary hashes and fixture versions are in the matching provenance JSON files. The accessibility review opened reasoning, inspected a command, opened changed-file diffs and scrolled to sources. The reasoning state was verified as expanded. Main response captures have the stated target dimensions; the initial home view is omitted.

| Surface | Light | Dark, narrow |
| --- | --- | --- |
| Chronological activity | [Activity](light-conversation-top.png) | [Activity](dark-narrow-conversation-top.png) |
| Final prose and code | [Answer](light-conversation-answer.png) | [Answer](dark-narrow-conversation-answer.png) |
| Expanded command | [Command](light-command-expanded.png) | [Command](dark-narrow-command-expanded.png) |
| File review | [Files](light-file-review.png) | [Files](dark-narrow-file-review.png) |
| Sources | [Sources](light-sources.png) | [Sources](dark-narrow-sources.png) |
| Expanded reasoning | [Reasoning](light-reasoning-expanded.png) | [Reasoning](dark-narrow-reasoning-expanded.png) |

The narrow view preserves the same content and uses scrolling for longer command/diff panels. The setup hint is visible because this fixture intentionally has no ready model. It does not represent a live model run. These images establish deterministic rendering and interaction, not comparative generation quality or live-stream frame timing.

Reproduction requires Linux, Xvfb, the existing Python AT-SPI/Pillow dependencies and a freshly built embedded binary:

```bash
npm run tauri -- build --debug --no-bundle
node scripts/native-response-review.mjs .local/qa/my-response-review
LOCALLM_REVIEW_THEME=dark LOCALLM_REVIEW_WIDTH=760 LOCALLM_REVIEW_HEIGHT=560 node scripts/native-response-review.mjs .local/qa/my-dark-review
```

Use a fresh output directory. `LOCALLM_XVFB_BIN` can point at an unpacked Xvfb binary. The review uses its own X11/D-Bus sessions and data directories; it does not need a Vite server or browser. `LOCALLM_REVIEW_HISTORY=100` adds 100 earlier turns for a native long-history smoke check. Recorded interaction times include accessibility overhead and fixed waits and are not a frame-rate benchmark.


## Long-history acceptance

The same embedded build completed the rich response review after loading **100 earlier turns plus the current turn**. Reasoning expansion was confirmed through native accessibility state and captured in [the long-history frame](long-history-reasoning-expanded.png). Source/binary identities and measured interaction values are retained in [provenance](long-history-provenance.json) and [interaction data](long-history-interaction.json).

The observed conversation-open check took 2,728.9 ms including the scripted one-second wait and traversal of the accessibility tree; disclosure action/state reading took 113.1 ms including a fixed 100 ms wait. These are native smoke-check timings with measurement overhead, not React render timings, latency percentiles or sustained streaming benchmarks.
