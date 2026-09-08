# Desktop lifecycle

LocalLM registers Tauri's single-instance plugin before other plugins. A second launch exits and asks the existing main window to show, unminimize and focus. Arguments from the second process are ignored; they do not initiate model or tool actions. This prevents ordinary duplicate launches from creating a second runtime manager or opening the same SQLite store in another application instance.

The window-state plugin persists size, position and maximized state in `.window-state.json` in Tauri's application configuration directory. Visibility, decorations and fullscreen are not persisted. A minimized window reopens normally. The upstream plugin checks available monitors before restoring saved positions; disconnected-monitor behavior still needs a physical multi-monitor acceptance check.

Versions are locked in Cargo.lock: single-instance 2.4.4 and window-state 2.4.1. These are native-only integrations and add no frontend IPC permissions. See the official [single-instance setup](https://v2.tauri.app/plugin/single-instance/) and [window-state documentation](https://v2.tauri.app/plugin/window-state/).

`scripts/window-smoke.ps1` verifies real Windows bounds restoration, maximized restoration, second-process exit, reuse of the original process and restoration of its minimized window. It restores the original window geometry afterward and leaves the app running. The script expects the debug app to be closed before starting and the Vite server available for its WebView. Foreground activation is requested but Windows can restrict focus stealing; the test verifies restoration, not a guarantee that every launch takes keyboard focus.

The existing shutdown handler cancels generation and stops the managed inference runtime. Installer testing, interrupted shutdown scenarios and a complete release acceptance audit remain outstanding.
