# Workspace panels

The inspector launcher offers Review, Terminal, Browser and Editor. The plus menu creates additional terminal and browser tabs. Ctrl+Shift+G, Ctrl+backtick, Ctrl+T and Ctrl+P select the panels. Arrow keys navigate focused workspace tabs. Escape hides the inspector unless the editor or terminal owns the key.

## Editor

Choose a workspace folder, then browse or filter the explorer. The action bar creates a file/folder in the selected directory (or the selected file's parent). Root clears selection. Rename and Delete act on the selected entry; deletion always shows the exact path and requires confirmation, including recursive folder deletion.

Documents use CodeMirror with line numbers, language highlighting, indentation, undo/redo, find/replace and Ctrl+S. The engine loads on demand. Tabs retain content, selection and undo history when switching tabs, hiding the panel or visiting Settings. Unsaved tabs show a dot. Closing a modified tab or the desktop window offers Save, Discard and Cancel. A failed save keeps the tab and draft open.

Native editor commands are separate from assistant tools. They bind each operation to its requested open workspace, validate relative paths, and use capability-relative filesystem access. Users can edit project configuration such as `.env`; this does not grant the assistant access to credential files. Binary, non-UTF-8 and oversized files show an error with an external-open action. The limit is 2 MiB. Linked paths are not edited or recursively entered.

Reads return complete text and SHA-256. Saves compare the expected hash before writing and again before replacement, sync a temporary file in the same directory, preserve file permissions, and atomically replace the destination. A conflicting external edit leaves the draft open; Reload requires confirmation before discarding it. Normal LF/CRLF line endings and UTF-8 BOMs are preserved. Mixed line endings normalize to the detected style after editing. Creation never overwrites an existing entry. Renames use atomic no-replace operations on Linux/macOS and Windows.

Drafts are retained in memory during the application session. Crash recovery, editing files beyond the size limit and non-UTF-8 conversion are not implemented.

## Review, Terminal and Browser

Review shows the branch, changed files and colored tracked diffs, including staged changes before the first commit. Changed files open in Editor. Refresh and window focus update the view. Non-Git folders show an empty state; branch switching never forces a checkout or discards disk changes.

Stage/Unstage acts on selected whole files. Commit previews the staged changes and requires explicit confirmation; a staged-tree hash detects changes after review. Unborn repositories are supported. Commits require the workspace to be the repository root. Create/switch branches and inspect recent history from Review, also reachable through Git settings. Native Git uses literal pathspecs, output/time limits and explicit errors; no history rewrites or automatic remote actions are performed.

Each Terminal tab owns a real PTY shell, initially in its workspace folder. Sessions and bounded scrollback survive panel switching. Closing a tab terminates its shell. Clear and Restart are available in the toolbar. Live output and replay are ordered under one lock, and split UTF-8 sequences are decoded across chunks. This user shell runs with the user's normal system permissions.

The PTY uses the shell configured in Bash settings. Closing also terminates the foreground process group on Unix. One output chunk is in flight until xterm acknowledges parsing it; the native backlog is bounded and emits a skipped-byte notice under overload. The manual terminal is separate from model command permissions and the model's sentinel-based persistent command session.

On Linux, the main WebKit view stays in a GtkOverlay and browser views use a separate fixed layer. Opening a browser therefore preserves the main application viewport instead of stacking and dividing the window. The browser layer applies its host bounds on allocation and remains separate from the main window’s size negotiation.

Browser accepts HTTP/HTTPS addresses, localhost URLs and search terms. It preserves native page state per tab, waits for surface creation before navigation, serializes show/hide/close operations, and hides native overlays under menus and dialogs. External navigation and popups share the URL validation boundary. Embedded web pages cannot invoke LocalLM application commands or inherit the main webview's plugin capabilities.

Native browser panels reveal immediately because a child webview cannot follow a DOM parent's CSS slide or fade. Layout updates coalesce to the latest bounds, include position-only changes, and skip unchanged rectangles. Native operations are ordered across tabs. If an ancestor animates, the page stays hidden until motion finishes and its final bounds are available; zero-sized hosts never create a tiny native surface.

## Assistant edits and formatting

The assistant's file tools are separate from the editor's. They write through
recoverable transactions: a multi-file edit is staged, journalled and
committed as a unit, and a failure rolls the whole transaction back.

**Edit matching is tolerant.** An edit finds its target in three passes —
exact bytes, then typographic variants (smart quotes, en/em dashes, exotic
spaces), then trailing whitespace ignored per line. `replace_all` replaces
every occurrence instead of requiring a unique one. The replacement reuses the
line endings of the region it replaces, so a CRLF file stays CRLF and
untouched bytes are never rewritten.

**Writes are formatted.** After a write, edit or patch, the project's
formatter runs on the file: `rustfmt`, `prettier`, `biome`, `gofmt`,
`clang-format`, `ruff`, `black`, `gofumpt`, `shfmt`. A formatter applies when
the file's final extension matches, the project uses it (`prettier` needs a
`prettier` dependency in the nearest `package.json`), and its binary is
installed. Formatting never fails a write: a broken toolchain is reported in
the tool result and the file keeps the bytes it was given. `format_paths`
formats files on demand.

Every mutating tool requires an explicit `expected_sha256` pin, and the
reported hash is the hash **after** formatting, so the next edit's pin lines
up with what a read returns.

## Undo

The files a conversation has touched are captured before each model step, so
a bad edit can be reverted without asking the model to redo the work. See
[snapshots and undo](SNAPSHOTS.md).

## Verification

- `npm test`: component and document lifecycle coverage, including save failures, concurrent saves, workspace isolation, tab/session retention and cancelled reloads.
- `npx playwright test e2e/editor.spec.ts`: the real CodeMirror engine with a simulated native API; covers files beyond 500 lines, CRLF saves, retained undo/drafts, create/rename/delete, conflict recovery, find and responsive layouts.
- `npx playwright test e2e/browser-motion.spec.ts`: frame-by-frame native host checks with a simulated API, covering reveal, expansion, position-only movement, ancestor animation, delayed layout, tab switching, narrow windows and reduced motion.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib editor::tests`: actual filesystem lifecycle, stale hashes, path validation, links, size/binary limits and Unix executable permissions.
- `npm run test:browser-layout` (Linux with Xvfb; set `LOCALLM_XVFB_BIN` if needed): a real Tauri/WebKitGTK window, loopback page navigation, pointer input in both views, exact browser bounds, every main-window allocation during browser creation/switching, tab visibility, resize, restore and close. This runs on its own display without opening user data.
- Rust `workspace_ui::tests`, `terminal_ui::tests` and `inspector_browser::tests`: temporary Git repositories, PTY termination and Unicode handling, and browser URL restrictions.
- `npm run build` and `cargo build --manifest-path src-tauri/Cargo.toml`: frontend and Linux native build checks.

Browser React tests use a native API fixture; the separate Linux native check exercises real child webviews and GTK allocation. As of 2026-10-16 the full Rust suite passes 432 tests with five opt-in/platform ignores, including previously failing credential/interpreter regressions. Windows/macOS native acceptance and installer verification remain separate release checks. See [the current full implementation report](IMPLEMENTATION_REPORT.md).
