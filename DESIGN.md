# LocalLM — design system: Darkroom

The app is a darkroom working under a safelight. People sit in it for hours in a dim room, next to an editor. Warm umber-black layers carry the structure. One safelight accent marks only two things: **where you are** and **what is live**. In light mode the lights come on, and the surfaces become warm fiber paper ("Print").

Color strategy: Restrained. Use neutrals plus one accent. The accent never fills a large area and never decorates.

## Rules of the world
- **The safelight rule.** `--accent` is used only for: the current location (active chat dot, active settings rail item marker, selected swatch ring), live or running state (pulse, running tool, "Working for…"), keyboard focus, the caret, the text selection tint, link underlines, and the checked switch. Nothing else gets it.
- **Label once.** Never show the same fact twice on a screen. A count, a title, or a model name appears in one place only.
- **Timer numerals.** Durations, token rates (PPS/TTFT/TGS), counts and times use `font-variant-numeric: tabular-nums`. Durations and rates also use `--font-mono` (they are measurements). Mono is not used anywhere else outside code.
- **Contact sheet sidebar.** Rows are dense and flat. The active row gets the `--active` fill plus a 6px safelight dot at its leading edge. Side borders are never coloured.
- **Live proof.** Appearance settings render a live proof (a miniature of the app) that reacts to every control immediately. Every control maps to a CSS custom property or `data-*` attribute on `<html>`, so changes need no reload.

## Tokens (`src/tokens.css`)
Surfaces rise in lightness: `--sidebar` < `--background` < `--surface` < `--elevated` < `--overlay`. Code uses `--code-bg`. Text: `--text`, `--secondary`, `--muted` (≥ 4.5:1 on `--background`).

### Stocks (neutral palettes; `html[data-stock]`)
Columns, in order: sidebar, background, surface, input, elevated, overlay, code-bg, border, border-strong, hover, active, text, secondary, muted, chat-user-bubble.

| Stock | Mode | Values |
|---|---|---|
| `umber` (default dark) | dark | #110f0d #161310 #1d1915 #1a1612 #231e19 #29231d #100e0c #29241e #3a322a #211c17 #2b251f #efe7db #b8ad9e #8f8576 #262019 |
| `graphite` | dark | #161615 #1b1b1a #222221 #1f1f1e #282827 #2d2d2b #171716 #2b2b29 #3a3a37 #272726 #30302e #ecebe8 #b0afa9 #8a8983 #2a2a28 |
| `ink` | dark | #0e1014 #12151a #191d23 #161a1f #1f242b #252a32 #0d0f12 #232830 #333a44 #1b1f26 #242a32 #e6e9ee #a9b0bb #838b97 #20252d |
| `pitch` | dark | #000000 #050505 #0e0e0e #0a0a0a #151515 #1b1b1b #000000 #1e1e1e #2e2e2e #141414 #1f1f1f #ededed #b0b0b0 #8a8a8a #181818 |
| `fiber` (default light) | light | #efe9df #f7f3ec #fdfbf7 #fffdf9 #fffdf9 #ffffff #efeae1 #e3dccf #d2c9b9 #ebe5da #e2dacd #1f1a14 #574e43 #6e6558 #ebe4d8 |
| `glossy` | light | #f2f2ef #fafaf8 #ffffff #ffffff #ffffff #ffffff #f4f4f1 #e6e5e0 #d3d2cc #efefeb #e6e6e1 #1d1d1b #55554f #6f6f68 #efefeb |
| `cool` | light | #eef1f5 #f6f8fb #ffffff #ffffff #ffffff #ffffff #eef1f5 #dfe4eb #cbd2dc #e9edf2 #dfe5ec #161a20 #4d5562 #636c79 #e8edf3 |

`--foreground` mirrors `--text`. `--focus-border` = `color-mix(in srgb, var(--accent) 45%, var(--border-strong))`.

### Safelights (accent; `html[data-accent]`)
| Safelight | Dark `--accent` / `--accent-strong` | Light `--accent` / `--accent-strong` |
|---|---|---|
| `amber` (default) | #f08a3c / #ffab6e | #b2530f / #8f420a |
| `ruby` | #ff7a5c / #ff9e86 | #c0391f / #9e2c15 |
| `sodium` | #e9c64a / #f5d978 | #8a6d00 / #6e5600 |
| `cyan` | #5cc8d6 / #8adbe5 | #0f7d8c / #0b6672 |
| `iris` | #8ea4ff / #b3c1ff | #3f5fd6 / #2f4bb8 |
| `custom` | `--accent-custom` / mixed 75% with white | `--accent-custom` mixed 72% with black / 58% with black |

`--accent-subtle` = accent at 9% over transparent, and `--accent-soft` = accent at 16%. Both are derived with `color-mix`, so presets only set the two accent values.

### Semantic
Dark: `--success` #6cc08f, `--warn` #e3b94f, `--error` #ec7f76, `--error-bg` #34201e. Light: #2f8a57, #8f6a00, #b8433a, #fbecea. Each has a `-subtle` fill at about 10%. Warn is yellow, so it stays distinct from the orange amber safelight. Full access is always shown in `--warn`, with its icon and label.

### Scale variables (set inline on `<html>` by `applyAppearance`)
- `--ui-scale` (0.9 / 1 / 1.1 / 1.2) multiplies the type scale: `--text-xs` 11.5, `--text-sm` 12.5, `--text-md` 13.5, `--text-lg` 15, `--text-xl` 20, `--text-2xl` 24.
- `--chat-font-size` (13–19px, default 15) and `--chat-line-height` (1.45–1.9, default 1.7) control transcript prose and the composer.
- `--font-chat` (sans / reading serif `'Sitka Text', Charter, 'Iowan Old Style', Georgia, serif` / mono), `--font-mono` (code font choice, ahead of the existing fallback stack), `--code-ligatures` (`normal` / `none`).
- `--chat-width` (narrow 640px / standard 760px / wide 920px / full `100%`).
- `--radius-scale` (sharp 0.4 / default 1 / round 1.5) multiplies the radii 5 / 7 / 10 / 14 / 18. Pills stay pills.
- Density (`html[data-density]` compact / comfortable / spacious): `--row-h` 26 / 30 / 36px, `--turn-gap` 28 / 40 / 56px, `--row-pad-y` 8 / 12 / 16px.

### Motion
`--dur` .15s, `--ease` cubic-bezier(.22, 1, .36, 1). `html[data-motion=reduced]`, or `system` combined with `prefers-reduced-motion: reduce`, removes transitions and animations. The one authored moment is the live pulse on running work.

### Contrast
`html[data-contrast=high]` sets `--secondary` to `--text` mixed 88% over `--background`, `--muted` to 74%, and `--border` to the `--border-strong` value.

## Patterns
- **Settings row** (`SettingRow`): label and description on the left, control on the right, inside a bordered `SettingsGroup` card. Use `stacked` for wide inputs. Vertical padding follows `--row-pad-y`.
- **Radio cards** (`RadioCards`): for mutually exclusive modes, with an optional badge. A warning tone marks risky options.
- **Switch:** a native checkbox with `className="switch"`.
- **Segmented control:** for 2–4 short choices.
- **Swatch picker:** a radiogroup of 28px circles (or stock chips that show their own background, surface, and text). The selected swatch gets a 2px `--accent` ring with a 2px gap. Each swatch has an accessible name.
- **Agent work:** one collapsed line, `Worked for 1m 39s · N steps`, above the final answer, with the duration in mono numerals. When expanded, it becomes a timeline with a 1px rail. Copy and PPS/TTFT/TGS stats appear once, under the final answer, in mono tabular `--muted`.
- **User message:** `bubble` (default) is a soft `--chat-user-bubble`, right-aligned, max 80% width. `plain` is left-aligned with no fill, `--secondary` text, and a hairline above the turn.
- **Tables in answers:** horizontal hairlines only, no vertical rules. The header row is `--secondary` at weight 600, and numbers are tabular.

## Don'ts
No eyebrow labels, gradient text, glow shadows, coloured side borders, or nested cards. Same-size icon cards are not used as page structure. Don't use emoji or Unicode glyphs as icons (use lucide). Don't use the safelight outside the safelight rule.

## Files
`tokens.css` (tokens, stocks, safelights) → `styles.css` (base, primitives) → `workspace.css` (shell, sidebar, transcript, trace, composer, inspector) → `models.css` → `settings.css` (settings shell, rows, appearance page). `src/lib/appearance.ts` owns the appearance model, persistence (`locallm-appearance`, migrating legacy `locallm-theme`), and `applyAppearance`.
