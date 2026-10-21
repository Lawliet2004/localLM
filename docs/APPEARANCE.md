# Appearance preferences

Open **Settings → Appearance**, or use **View → Appearance**. This separate settings page extends the existing warm paper, forest, and IBM Plex interface. Changes apply immediately on this device; there is no Save button.

| Setting | Supported values | Default and effect |
| --- | --- | --- |
| Theme | Light, Dark, System | Light. System follows OS color-scheme changes, including while the app is open. The native select and miniature theme previews stay synchronized. |
| Accent color | Forest, Ocean, Violet, Rose, Amber | Forest. Changes action, selection, and focus colors while retaining the theme's surfaces. |
| Conversation font | IBM Plex Sans, System font, IBM Plex Serif | IBM Plex Sans. Applies to message text and the composer; interface labels retain IBM Plex Sans. Plex fonts are bundled locally. |
| Text size | 12–20 px, whole pixels | 14 px for conversation text and the composer. |
| Code size | 11–18 px, whole pixels | 13 px for fenced code blocks, the editor, and terminal. Existing editor content and undo history are preserved when the size changes. |
| Interface density | Comfortable, Compact | Comfortable. Compact reduces spacing in settings, navigation, and conversations. |
| Conversation width | Standard, Wide, Full width | Standard. Composer limits are 824 px, 1080 px, and available width respectively; messages use the corresponding reading width with internal spacing. |
| Animations | Follow system, Reduce motion | Follow system. Either an OS reduced-motion request or Reduce motion suppresses CSS animations/transitions, makes Jump to latest instant, and disables terminal cursor blinking. |

The reading preview reflects conversation font, text size, and code size. Theme shortcuts in the rail and View menu select a fixed Light or Dark preference, replacing System. Narrow windows stack preference rows and use horizontal settings navigation.

**Reset appearance** restores every default above and saves them immediately. It is disabled when all defaults are active and there is no save error. Reset affects appearance preferences; conversations, drafts, model configuration, and permissions keep their existing storage paths.

## Persistence and recovery

- The complete preference object is stored as JSON in `localStorage` under `locallm-appearance`. Relevant storage events synchronize other windows sharing that storage. Startup applies saved preferences before React mounts.
- Missing or malformed JSON falls back to the older `locallm-theme` value, with defaults for the new settings. Each field in valid JSON is validated independently; unsupported choices and out-of-range or fractional sizes fall back to that field's default. Reading legacy data alone does not rewrite storage; the next change or reset saves the full object.
- Successful updates also attempt to write `locallm-theme` for compatibility. Failure of this compatibility write does not invalidate a successful full preference save.
- If storage cannot be read at startup, defaults are used. If a write fails, the change remains active for the session and Appearance displays an alert explaining that it could not be saved. Its footer also reports the unsaved state. A later successful change or reset clears the error.

## Implementation and verification

- `src/lib/appearance.ts` defines defaults, validation, migration, theme resolution, reduced-motion detection, and root CSS variables/attributes. `src/lib/useAppearance.ts` handles live changes, persistence, errors, OS theme tracking, and storage events; `src/main.tsx` applies startup preferences.
- `src/components/AppearanceSettings.tsx` and its stylesheet provide the controls and previews. `src/appearance.css` connects preferences to the established tokens and layout. `App.tsx`, `SettingsNavigation.tsx`, and `GeneralSettings.tsx` integrate the separate destination; `CodeEditor.tsx`, `TerminalPanel.tsx`, and `Chat.tsx` consume the relevant preferences.
- Run `npm test -- src/lib/appearance.test.ts src/components/TerminalPanel.test.tsx src/components/Chat.test.tsx` for validation, migration, persistence failures, OS changes, terminal updates, and reduced-motion scrolling. Run `npx playwright test e2e/appearance.spec.ts e2e/editor.spec.ts e2e/redesign.spec.ts e2e/settings-design.spec.ts e2e/providers.spec.ts` for browser integration, reload/reset, draft retention, font loading, editor history, and responsive settings. `npm run build` checks TypeScript and the production bundle.
- Appearance browser tests generate the desktop/mobile light/dark captures and `appearance-details.png` under `.impeccable/review/`. Browser preview covers preference behavior; a real terminal shell requires the desktop application.
