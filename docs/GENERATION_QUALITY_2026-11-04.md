# Generation quality: research and implementation plan

The target is better delivered websites, playable games, editable 3D assets, and working services. Better application chrome alone does not establish this outcome. Preserve the user's tools, project rules, source selection and approval policy.

## Research

Checked 4 November 2026. DeepSeek Harness release `dsh-v0.2.2-alpha.3` (3 November) and public source snapshot `8bd073392eef6f54e921a093b95076248eb08cbb` were examined. Its desktop system prompt, filesystem tool documentation, skill package and agent loop favor discoverable tools, progressive skill loading, bounded edits, collecting execution results and explicit background job lifecycles. The examined material does not establish a special universal website/game/3D quality prompt or measured superiority over LocalLM.

- [DeepSeek releases](https://github.com/deepseek-ai/deepseek-harness/releases)
- [Desktop prompt snapshot](https://github.com/deepseek-ai/deepseek-harness/blob/8bd073392eef6f54e921a093b95076248eb08cbb/snapshots/sdk/desktop-context/system-prompt.expected.md)
- [Skill implementation and catalog lifecycle](https://github.com/deepseek-ai/deepseek-harness/blob/8bd073392eef6f54e921a093b95076248eb08cbb/packages/skill/tool-skill/README.md)
- [Filesystem tool contracts](https://github.com/deepseek-ai/deepseek-harness/blob/8bd073392eef6f54e921a093b95076248eb08cbb/packages/fs/tool-fs/README.md)

Public evaluation references found at the user's request:

- [WebGen-Bench](https://github.com/mnluzimu/WebGen-Bench): multi-file websites and interactive functionality.
- [WebGameBench](https://arxiv.org/abs/2605.17637): evaluate the delivered game through input, rules, state transitions, terminal conditions and restart behavior.
- [Code3DBench](https://github.com/VladimirGl/Code3DBench): executable Three.js geometry with a fixed rendering scaffold and visual references.

These inform the local evaluation cases; no benchmark tasks, assets, scores, or success claims are imported. Local cases are original prompts. Build success, functional correctness, visual quality and user preference must be evaluated separately.

## Plan

1. Ship first-party, task-specific authoring guides for frontend, games, 3D and backend work. Offer compact guidance only for development requests, with full guides available on demand. Keep normal conversation and the stable system prefix unchanged.
2. Add a bounded, read-only `project_inspect` tool for actual manifests, scripts, entry points and assets. Reuse workspace containment, no-follow reads, tool contracts and source/capability policy. Do not execute discovered scripts automatically.
3. Replace the parent loop's write/check boolean with revision-aware verification tracking. Starting or failing a check must not count as verification; later edits and unrelated historical reports must not reuse an old pass. Retain one bounded correction before completion.
4. Add regression coverage for policy, prompt routing, scoped inspection, and check lifecycle. Provide reproducible, model-independent acceptance cases for website, game, 3D and backend generation.
5. Run library checks and frontend typecheck. Record what was actually verified. A same-model paired generation evaluation remains separate from harness unit tests; visual or model-quality parity requires real generated artifacts and authorized runtime inspection.

## Constraints

No model training or weight fine-tuning is performed. These are harness changes. Never claim a renderer, preview, test, export or server worked without corresponding evidence. Respect the user's existing refusal of localhost browser preview. Do not restore a composer response-presentation selector. Do not request or store API keys in this document or in prompts.

## Results

Implementation and verification results will be recorded after the changes are exercised.
