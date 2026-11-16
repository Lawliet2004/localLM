# LocalLM output-quality improvement plan

Date: 5 November 2026. Status: proposed; implementation is not authorized by this planning request.

## Objective

Improve the artifacts LocalLM delivers: websites with a coherent visual direction and working interactions, usable/editable 3D assets, and correct maintainable code. Chat appearance is a separate concern. A longer system prompt, a clean build, or a more confident final answer cannot establish artifact quality.

## What the current source establishes

- `authoring.rs` already provides project inspection, development-guide loading, and keyword-based task hints. Frontend and 3D guides already describe sensible implementation and inspection practices. Additions should extend these, not introduce another competing prompt stack.
- `chat.rs` injects guidance and uses revision-aware `verification_progress::Progress`. The completion path has a bounded verification reminder, not an enforced artifact-quality workflow.
- `verification_progress.rs` tracks current edits, pending checks and failed checks. It accepts a successful command as evidence for that revision. This does not represent separate requirements for build, functionality, appearance and export validity. A passing build cannot expose an untested requirement that was never registered.
- `project_inspect` returns bounded manifests and asset paths; it explicitly does not inspect the rendered appearance or establish runtime availability.
- `development-acceptance.py` tests development infrastructure. It is not an end-to-end generation-quality benchmark.
- The preceding River smoke test proves that the supplied model can stream a response, not that it can deliver high-quality artifacts.

Likely causes to investigate: weak task/reference context, insufficient visual feedback, unavailable asset/modeling capabilities, premature completion, model limitations, or lost context across iterations. These are hypotheses, not a measured ranking of causes.

## Delivery order

### 1. Establish an artifact baseline before changing model behavior

Create 18 original fixed tasks: six websites, six 3D deliverables, six code tasks. Include greenfield and existing-project work, narrow layouts, realistic content, error states, export requirements, bug fixes and tests. Keep a separate unseen set for final evaluation.

Run each task with the same model, sampling settings, tool availability, initial repository, and token/time budgets. Repeat each task three times to expose variability. Begin with one task per domain to validate the evaluation runner before paying for the full baseline.

Archive source revision, prompt, available tools, model settings, tool trace, final files, completed checks, screenshots/renders when permitted, costs where reported and elapsed time. Redact credentials. Record blocked capabilities separately from model failures and show both their rate and their effect on delivery.

Score separate dimensions: brief coverage, functional correctness, visual quality, maintainability/editability, and delivery completeness. Use deterministic checks for behavior and blinded human comparisons for appearance; a model critic is supplementary. Never average away a broken primary journey or invalid export.

Deliverable: reproducible benchmark runner and a failure report identifying the three most common causes of weak output.

### 2. Introduce a durable task contract and capability preflight

Before substantial work, record the requested deliverable, audience, existing stack, constraints, references, required files and observable acceptance criteria. Derive routine decisions from the repository and request; ask only about choices that materially change the result.

Detect actual capabilities: source read/write, command execution, browser/runtime observation, model image input, asset generation/acquisition, and 3D authoring/render/export tools. Provider names alone are not proof of capability. If visual input or an execution route is unavailable, record the limitation before planning around it.

Persist the contract and decisions in the session journal. Follow-up requests such as “make it more polished” must inherit project context rather than depend on matching English keywords again. The contract informs tool discovery; it never grants permissions.

Each acceptance item has an ID, category, relevance, status and evidence references. Distinguish passed, failed, not checked, blocked and explicitly waived. Bind evidence to the source/runtime configuration it observed. Later edits invalidate affected evidence; use conservative invalidation where dependencies are unknown.

Acceptance: an app that builds but has a broken checkout is incomplete; a declined visual check is visibly unverified, not silently passed. Users may still receive partial work with accurate limitations.

### 3. Build an evidence-driven review-and-repair loop

Use a lightweight persisted sequence: inspect → define acceptance → implement → run → inspect artifacts → identify defects → repair → recheck → deliver.

Do not force a heavyweight planning ritual onto simple edits. For substantial artifact work, review the actual result. Each defect must identify an affected requirement, concrete observation, severity and a proposed correction. The next edit targets that defect; avoid full rewrites without evidence.

Initially allow up to three repair cycles within a configured run budget. Stop early when required checks pass and no high-severity defects remain. Stop on budget exhaustion, repeated identical failure or missing capabilities, retaining partial artifacts and an honest status. Track the best validated revision in an isolated worktree where appropriate; never reset a user's dirty tree.

Start with one agent and a distinct review pass. Evaluate an independent critic or specialized agents only after the single-agent loop is measurable. They add latency and cost and are not inherently better.

Acceptance: interrupted runs resume with the same criteria and evidence; a repair cannot reuse stale validation; final claims reference actual reports and artifacts.

### 4. Add domain-specific production paths

#### Websites

- Establish a brief visual direction before coding: composition, hierarchy, typography, spacing, palette, content density and imagery. Preserve an existing product's design system.
- Integrate available design skills such as Impeccable through the existing skill loader, with compact discovery and task-specific loading. Skills guide decisions; they do not prove output quality.
- Inspect a small number of relevant visual references when authorized. Extract useful principles rather than copying a whole site or inserting generic card layouts.
- Inventory usable assets. Offer authorized image generation or acquisition when visuals materially affect quality; preserve source/licensing information. Do not pretend placeholder art meets a realistic-image requirement.
- Implement the primary user journey before polish. Exercise every important control, validation failure, empty/loading state and persistence boundary.
- Use an authorized preview/observation adapter to capture wide and narrow layouts, inspect keyboard behavior and console errors, and test interactions. Link captures to route, viewport, state and source revision.
- Feed actual screenshots to a vision-capable reviewer when available. Require specific layout/hierarchy/content defects rather than “make it modern.”

Acceptance: a working primary journey, no critical overflow or console errors in tested states, usable keyboard focus, and a human-reviewed composition. Preview refusal remains authoritative; such runs cannot claim visual acceptance.

#### 3D assets and scenes

- Separate an editable model, real-time scene, game asset and rendered image. Their outputs and checks differ.
- Route to installed, authorized tooling suited to the deliverable. For example, an editable Blender asset needs an actual modeling/export path; an interactive browser scene needs a real scene runtime. Do not assume either is installed.
- Define silhouette, proportions, units, materials, topology/UV needs, polygon budget and export format. Organic/sculptural work may require specialist asset-generation tools or supplied assets; identify that limit instead of substituting primitive shapes without agreement.
- Build and inspect a blockout first. Check front, side and perspective views before spending effort on details and materials.
- Add materials, lighting and camera framing, then render multiple views. Examine normals, clipping, intersections and scale. Apply topology requirements according to purpose; manifold geometry is not a universal requirement for every scene.
- Reimport the exported file in a real importer. Check bounds, mesh/material counts and required external resources. Deliver native editable files plus requested exports and previews.

Acceptance: recognizable requested form, valid usable export, appropriate editability, and inspected multi-view renders. A screenshot alone never counts as a 3D model.

#### Code

- Build a scoped repository map from manifests, architecture, relevant call sites, tests and applicable instructions. Preserve existing APIs and conventions.
- Translate requirements into behavioral checks before implementation; prioritize critical edge cases instead of tests that merely repeat the code.
- Use small coherent changes, diagnostics and relevant execution. Test the complete integration boundary, not only mocked helpers.
- Review the final diff for incorrect assumptions, missing error handling, unnecessary dependencies, regression risk and accidental scope expansion.
- Preserve commands, test counts, failures and source freshness. A command that finds no tests does not count as passing tests.

Acceptance: observed required behavior, relevant regression checks, no new blocking diagnostics, and a reviewable scoped diff.

### 5. Evaluate model and context strategy against the same artifacts

Keep the current River model as the baseline. Compare available models under identical tools and budgets only after the observation loop works. Evaluate both a fixed-model improvement and any extra-compute variant so gains are not misattributed.

Measure whether a stronger implementation model, vision reviewer, or targeted escalation actually helps each domain. Escalate after concrete repeated failure or task complexity, within explicit cost/provider constraints. Do not silently send private project material to another provider.

Maintain a compact durable context: acceptance criteria, design decisions, relevant file map, current defects and evidence pointers. Load detailed logs/assets on demand. Avoid repeatedly sending huge tool catalogs or dropping the design brief during compaction.

Acceptance: choose routing from measured success/cost/latency tradeoffs, not model branding. Record unsupported vision/tool behavior and context truncation.

### 6. Roll out incrementally and retain regressions

Ship the website path first, then 3D, then broaden code cases, while retaining cross-domain regression coverage. Treat every failed real task as a candidate regression fixture after sanitization and user-data review.

Suggested pilot targets, to revisit after baseline:

- At least 90% completion of required functional checks on the controlled pilot tasks.
- At least 70% blinded visual preference for revised website/3D results against baseline; report ties, sample size and uncertainty.
- All accepted 3D outputs parse and reimport with the required resources.
- Every “verified” claim has fresh matching evidence; no fabricated success claims.
- Median and tail latency, token usage and cost remain visible. Compare quality at equal budgets and separately show gains purchased with extra budget.

These are proposed release gates, not achieved scores. Small pilot samples cannot establish universal superiority.

## First implementation milestone

One website request should produce a saved acceptance contract, an actual working artifact, authorized visual/interaction evidence, a concrete defect list, a targeted repair and a source-bound delivery report. Run that same task before and after with the current model. In parallel conceptually—not a request to spawn agents—retain one 3D export and one code regression task to ensure the design remains general.

Implementation touchpoints: extend `authoring.rs` and its guides for context/criteria; extend `verification_progress.rs` from revision bookkeeping to categorized evidence; integrate bounded stages in `chat.rs`; reuse journal/artifact facilities for persistence and captures; keep tool contracts in `tool_spec.rs` and normalized results at existing boundaries. Create a separate artifact-quality benchmark runner rather than relabeling infrastructure tests.

## Constraints and non-goals

This plan does not authorize preview, dependency installation, paid services, model routing, publishing or permission expansion. Existing localhost-preview refusal remains in force; visual acceptance must use an authorized environment or remain blocked. Preserve the dirty working tree, tool-envelope contracts and Settings-only response presentation. Keep secrets out of prompts, logs, fixtures and reports.

Do not start with a large multi-agent rewrite, an enormous system prompt, hard-coded website templates, or a cosmetic chat redesign. First establish where quality fails, then prove that an artifact observation-and-repair loop improves it.
