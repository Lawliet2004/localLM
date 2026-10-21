# Browser and computer use

The local model can control a separate browser or the primary desktop monitor through the same tool approval and audit path as workspace tools. Both capabilities start disabled. They require the native app; browser preview only displays their settings.

## Setup

1. In **Settings → Execution**, detect or select your installed Node.js executable and save the interpreter settings. Node.js must include npm.
2. Open **Settings → Browser Use** and choose **Set up browser engine**. This installs exactly `@playwright/mcp@0.0.83` under the app data directory's `automation/runtime`, with npm lifecycle scripts disabled. It does not enable browser control. Chrome or Microsoft Edge must already be installed.
3. Choose Chrome or Edge, enable **Browser use**, and choose **Save browser settings**. Use **Browser paths** to choose a custom browser executable or an existing package installation. The package folder must contain the matching `package.json` and `cli.js`.
4. For desktop control, open **Settings → Computer Use**, enable **Computer use**, choose **Save computer settings**, and select a model that supports image input. A local model needs a vision projector; a provider model needs its image-input capability configured.
5. In the conversation's **Tools** panel, select **Browser use** or **Computer use**. Standard, Research, Coding, Code, and Creator profiles offer automation. Chat and Minimal do not.
6. Use **Check browser setup** or **Check computer setup** on the corresponding settings tab for dependency and desktop-session diagnostics. This checks package files and display availability without capturing the screen or sending input. The first approved browser call verifies that the browser itself can launch.

If automatic npm discovery fails, install the package manually and choose its folder:

```sh
npm install --prefix /absolute/path/to/browser-runtime --ignore-scripts --save-exact @playwright/mcp@0.0.83
```

Choose `/absolute/path/to/browser-runtime/node_modules/@playwright/mcp`. No source checkout or project `node_modules` is required by the installed application. The browser tool schemas are embedded at compile time. Native desktop drivers are compiled into the app.

## Usage and behavior

Try “Open example.com and report its main heading,” or “Fill the name field on this page, then wait for me before submitting.” Browser tools expose navigation, accessibility snapshots, click, hover, fill/submit, select options, keys, scrolling, screenshots, tabs, and close. Use fresh snapshot target references after navigation. The upstream JavaScript evaluation, arbitrary code, file upload, file-output arguments, browser-extension, and CDP attachment features are not exposed.

Desktop tools expose screenshot, pointer position, move, click, Unicode text, key combinations, and scrolling. Coordinates are relative to the primary monitor. Screenshots report original monitor dimensions; the image can be downscaled to a 1600-pixel longest edge, so map image coordinates proportionally to the reported dimensions. Typing affects the focused application. Keys are released within each call.

Ask for approval and Auto-approve reads require approval of every automation call, including screen captures and pointer reads. Full access allows selected calls without prompts. Enabling a capability does not select it for every conversation. Automation is excluded from inherited subagent calls; the parent conversation must perform those actions.

Each conversation owns an isolated in-memory browser profile. It does not attach to your personal browser or reuse its cookies. At most eight browser sessions are retained; the browser closes after five minutes idle. **Reset browser sessions** closes retained sessions and clears their temporary output directories. Deleting a conversation closes its browser. App exit closes all browsers. An interrupted or timed-out active browser call drops its process tree and invalidates that session. An action that has already reached a website or desktop app cannot be undone by cancellation; inspect state before retrying.

Screenshots are saved as conversation-owned PNG artifacts in the local database. Tool history contains image references, and **View screenshot** loads an image preview. Vision models receive the two latest screenshots as image inputs after complete tool batches. Raw base64 is not placed in textual tool output. Remote preflight uses an estimated 8192-token allowance per image; actual provider usage remains authoritative. Screenshot artifacts may be included in conversation exports, and are subject to the database's existing unencrypted-at-rest behavior.

Browser automation checks navigation targets and rejects local files, loopback, private-network and metadata addresses. It uses an isolated profile and the browser's ordinary security boundaries. Browser and desktop actions are not OS sandboxes. Use the scoped workspace preview tool below to open generated local HTML projects.

## Platforms and development dependencies

- **Windows:** Enigo input and XCap capture are compiled in. Elevated windows and protected desktops can reject input/capture. Windows execution and installer acceptance still need validation on Windows.
- **Linux X11:** Enigo uses X11 input; XCap captures the primary monitor. A graphical session and its display access are required.
- **Linux Wayland:** desktop input is rejected explicitly. XCap capture depends on compositor support and may require the desktop's screen-sharing dialog. Browser use still works independently.
- **macOS:** upstream drivers exist and may require Accessibility and Screen Recording permissions. This integration has not been validated on macOS.

Linux source builds need the project's existing Tauri/GTK development dependencies, plus PipeWire/SPA, XCB RandR, GBM and D-Bus development files. On Debian/Ubuntu:

```sh
sudo apt install libpipewire-0.3-dev libspa-0.2-dev libxcb-randr0-dev libgbm-dev libclang-dev libdbus-1-dev
```

This environment lacked those development headers and passwordless sudo. Validation used locally extracted development packages in `/tmp/sys-dev` and the installed runtime libraries, without changing system packages:

```sh
export PKG_CONFIG_PATH=/tmp/sys-dev/root/usr/lib/x86_64-linux-gnu/pkgconfig
export BINDGEN_EXTRA_CLANG_ARGS=-I/usr/lib/gcc/x86_64-linux-gnu/15/include
export LIBRARY_PATH=/tmp/sys-dev/libs
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --lib automation::tests
cargo test --manifest-path src-tauri/Cargo.toml --lib computer_use::tests
```

Those temporary paths are environment-specific; normal development installations use the system packages instead.

## Open-source provenance

| Component | Version | License | Source |
| --- | --- | --- | --- |
| Playwright MCP | 0.0.83 | Apache-2.0 | https://github.com/microsoft/playwright-mcp |
| Playwright / Playwright Core, exact dependency of that MCP release | 1.64.0-alpha-1790635538000 | Apache-2.0 | https://github.com/microsoft/playwright |
| Enigo | 0.6.1 | MIT | https://github.com/enigo-rs/enigo |
| XCap | 0.9.8 | MIT | https://github.com/nashaofu/xcap |
| image | 0.25.10 | MIT OR Apache-2.0 | https://github.com/image-rs/image |
| base64 | 0.23.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |

The selected MCP release depends on the listed Playwright alpha build; its precise version is fixed by the MCP package, and the managed installation retains npm's lockfile. Browser schemas are adapted from this release's `tools/list` response, restricted to the listed actions and input bounds. License texts are retained under `src-tauri/resources/automation/licenses` and included in Tauri bundles. Installed npm packages retain their own upstream license files. Cargo.lock pins the native dependency tree.

## Verification

The real-browser test starts a loopback fixture and exercises this integration's actual Rust → MCP process → Chrome path, including forms, click, snapshots, scrolling, screenshots, tabs, conversation isolation, in-flight cancellation, and cleanup:

```sh
LOCALLM_BROWSER_TEST_PACKAGE=/absolute/path/node_modules/@playwright/mcp \
cargo test --manifest-path src-tauri/Cargo.toml --lib automation::tests::real_browser -- --ignored --nocapture
```

The test is opt-in because Chrome and the pinned MCP package are external runtime prerequisites. Desktop unit tests validate schemas, input bounds, coordinate mapping, key parsing, and unsupported-session behavior without moving the real mouse or typing. The opt-in desktop integration check launches its own Xvfb display and a small X11 fixture, then verifies native pointer movement, position, clicks, typing, Ctrl+A, scrolling, and PNG capture against actual X11 events:

```sh
node scripts/test-desktop-automation.mjs
# Run browser acceptance separately, also on an isolated display:
LOCALLM_BROWSER_TEST_PACKAGE=/absolute/path/node_modules/@playwright/mcp \
node scripts/test-desktop-automation.mjs --browser
```

It requires Xvfb, a C compiler, X11 development files (`xvfb build-essential libx11-dev` on Debian/Ubuntu), and the source-build dependencies above. `LOCALLM_XVFB_BIN` can select a locally extracted Xvfb binary. The launcher supplies the isolated display only to its child test and terminates it on completion; it does not send input to the user's desktop. The ignored Rust test refuses to run without the launcher's explicit isolation flag and fixture path.

Diagnostics and the Wayland input rejection path were checked read-only here. Windows desktop input/capture and installer acceptance remain untested on this Linux host. No end-to-end model-driven desktop task is claimed.

Current regression evidence (2026-10-16): 509 frontend tests and all 34 Playwright tests pass; the complete Rust run passes 432 tests with five opt-in/platform ignores and no filtered tests. This supersedes the earlier baseline run that excluded failing modules. The opt-in browser and isolated native desktop checks both passed separately. Actual native WebKit layout and app startup/Settings smoke also passed. See [the full report](IMPLEMENTATION_REPORT.md) for commands and limits.

## Local HTML projects

`workspace_open_preview({"path":"tetris.html","browser":"chrome"})` opens a
saved workspace HTML project in a visible Chrome window (`msedge` is also
supported). It is an execution tool: select **Workspace** and **Local execution**
in the conversation. Browser and Computer automation and Playwright are not
required. Normal mutation approvals, execution enablement, always-ask settings,
permission rules and capability kills apply. Only the foreground conversation
can launch a preview.

The preview serves the HTML file's directory and relative static web assets from
an owned `127.0.0.1` endpoint. Every URL includes an unguessable session token;
requests must use the exact loopback host and port. It serves only regular,
non-hidden web assets up to 4 MiB each, with scoped no-follow file opens, no
traversal or listings, and no write endpoints. Assets are read again on refresh,
so saved file edits are reflected without reopening the preview. Absolute asset
paths starting with `/` do not carry the token; use relative asset paths.

A conversation owns one preview; reopening replaces its previous server. Up to
eight previews can be open. Servers expire after one hour, close when their
conversation is deleted, and stop on app exit. A launcher success reports
`launchRequested: true, pageVerified: false`: it does not certify that the page
loaded or the game was tested. Chrome's window is controlled by the user and
can remain open after its preview server closes.

Public-network Browser automation continues to reject arbitrary local files,
loopback/private-network targets and metadata addresses. This scoped preview
tool does not grant automation access to other local services.
