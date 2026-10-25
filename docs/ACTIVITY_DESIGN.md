# Activity design (flat, monochrome output log)

Everything the assistant outputs is styled by `src/activity.css`, imported last
in `src/App.tsx`. Removing that one import reverts the redesign; no older
stylesheet was edited. Selectors are prefixed with `:root` so they out-rank the
layered rules in `styles.css`, `workspace.css`, `desktop.css` and
`responses.css` (and the lazily loaded `genui/genui.css`) without `!important`.
The only `!important`s are for inline styles (ANSI dim spans, the chat error
banner's inline margin).

## Principles

- Text first. No card, pill, tinted panel or bordered box inside assistant
  output. Structure comes from alignment, weight, dimming and a dotted rail.
- One action = one line: verb, target (mono, truncated), status and time on the
  right. A collapsed row never wraps, at any width.
- Color is information. Black, white and gray only. Red (errors, deletions) and
  green (additions, success check) are allowed, and only for diffs, failures and
  success checks.
- Motion only means "something is happening": a shimmer on the verb of a
  running action and on `Thinking…`. No entrance animations, no hover lifts.
  `prefers-reduced-motion` and `:root[data-motion=reduced]` stop all of it.

## Tokens

| Token | Light | Dark | Use |
|---|---|---|---|
| `--a-ink` | `#111111` | `#f4f4f4` | verbs, answer text, active tab, focus ring |
| `--a-body` | `#2b2b2b` | `#d9d9d9` | command output, detail text, inline code |
| `--a-dim` | `#5c5c5c` | `#a6a6a6` | targets, secondary text, thinking text |
| `--a-faint` | `#9a9a9a` | `#6a6a6a` | rails and decoration only, never text |
| `--a-line` | `#cfcfcf` | `#3c3c3c` | hairlines |
| `--a-add` / `--a-del` | `#1a7f37` / `#c62828` | `#56d364` / `#ff7b72` | diff and status decoration |
| `--a-add-text` / `--a-del-text` | `#17692e` / `#b32222` | same as `--a-add`/`--a-del` | green/red **text** (darkened in light mode to clear 4.5:1) |
| `--a-hover`, `--a-code-bg` | translucent black / `#f4f4f4` | translucent white / `#161616` | inline-code fill, fenced-code surface |
| `--a-rail`, `--a-rail-x`, `--a-indent` | `1.5px dotted`, 8px, 20px | | rail geometry (7px / 12px under 600px) |

Contrast of ink, body, dim, add-text and del-text against the page, surface and
code backgrounds is at least 5.6:1 in both themes (computed, not eyeballed).

## Row anatomy

```
[icon 18px]  Verb  target (mono, dim, truncated) ............ [status] 2.1s
┊
┊ ╭ $ npm test
┊ │ output lines, bare monospace…
┊ ╰ Show all 214 lines
```

| Kind | Verb | Target |
|---|---|---|
| command | `Ran` / `Running` / `Run` | first line of the command |
| read | `Read` | filename, `#L10-40` when a range exists |
| edit | `Edited` | filename, then `+12 −3` |
| search | `Searched` | query, then `N results` |
| web search | `Searched the web` | query, then `N sources` |
| fetch / open | `Fetched` / `Opened` | domain + path |
| task | `Checked task` / `Task` | task name |
| other tool | tool name | first scalar argument |

Tools from a named local server keep `Server · tool` in the title so provenance
stays visible.

Status: running = spinner glyph and a shimmering verb; success = green check
and duration; failure = red `Failed · Exit N`; denied = red `Denied`; awaiting
approval = ink text with a dotted underline; interrupted = dim
`Stopped · outcome unknown`. Failed, running and awaiting rows open by default.

Expanded body, in order: path/meta, error text (red, no background), `$ command`,
web answer and numbered sources, then Output / Result / Diff / Code / Parameters
(text-link tabs, 1px underline on the active one; output capped at 340px).

## Answer text

Ink prose at `--chat-font-size`, line-height about 1.7. Links are ink with a
hairline underline. Headings differ by weight and size only. Blockquotes use the
dotted rail. Tables have no outer border or zebra. Inline code is the only fill.
Fenced code keeps a header (language, copy) on a flat gray surface with
grayscale syntax (`--syn-*` is remapped inside the output surface; keywords gain
weight, strings and comments dim). The streaming caret is an ink block.

## Rules for contributors

1. No new color literals in components. Use the `--a-*` tokens.
2. Do not wrap assistant output in a bordered or tinted container.
3. Faint is for rails only. Anything readable is `--a-dim` or darker.
4. Keep every existing class and `data-testid`; add classes rather than renaming.
5. Keep visible focus on every `summary` and `button` (2px ink outline).
6. Check both themes and widths 380 / 600 / 900.
