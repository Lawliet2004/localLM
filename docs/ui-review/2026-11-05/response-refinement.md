# Response activity refinement — 5 November 2026

This pass changes the response activity presentation in source. It is not native visual acceptance.

## Provider evidence

Read-only inspection of the application database found River at `https://api.river.ai/v1`, marked verified, with `deepseek-ai/DeepSeek-V4.1-Flash-1017` saved and selected as the preferred model. Its encrypted credential file exists. This pass did not decrypt or replace that saved credential.

A direct streaming Chat Completions request using the supplied River key and exact model returned HTTP 200, 16 choice chunks and `[DONE]` in 2.73 seconds. The prompt requested one sentence about stopping a loading indicator after completion. The answer was coherent and complete. The sample returned zero reasoning characters. This checks the supplied key and model, not the native application's saved credential or rendering path. No key was written to source or review artifacts.

## Design and changes

Applied the official [Impeccable skill](https://github.com/pbakaus/impeccable/blob/main/.agents/skills/impeccable/SKILL.md), its polish and craft-floor references, and the installed context/detector commands. Inspected the [assistant-ui reasoning demo](https://www.assistant-ui.com/elements/reasoning) with computer use, and consulted its [grouped activity guide](https://www.assistant-ui.com/docs/guides/chain-of-thought). The prior native capture `river-deepseek/baseline-live-000.png` was inspected as historical context, not as a current render.

- Shared reasoning disclosure replaces duplicate implementations. A stable Reasoning label accompanies three small circular dots; only active dots animate. Recorded duration remains available on completion. User expansion persists across streaming updates.
- Generic thinking/generation status uses dots visually while preserving accessible status text. Specific tool and preparation activity remains visible. Status text no longer pulses.
- Collapsed chevrons point right and expanded chevrons point down. Expanded reasoning has a quiet guide line and inset content.
- Assistant copy controls have enough width for their labels, stable visibility, and explicit hover/focus treatment.
- Colors use existing tokens. Both system and app reduced-motion preferences stop dot animation.

## Verification and limits

`npm run typecheck` passed. Focused ActivityTimeline, Chat and ToolControls tests passed: 95/95. `git diff --check` passed. The Impeccable mechanical scan returned no findings; it is not proof of visual quality.

Computer use exposed Chrome but no native app control. The repository excludes localhost browser preview, so no alternate preview or desktop automation was used. Fresh LocalLM captures, native expansion/toolbar interaction, light/dark/narrow comparison, and live reasoning rendering remain unverified. The installed desktop binary was not rebuilt or replaced. Next acceptance step: run this frontend in LocalLM with native computer control, generate a real response, and compare active, expanded and completed states in both themes.
