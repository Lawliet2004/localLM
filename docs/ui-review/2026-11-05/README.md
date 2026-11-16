# Embedded conversation review — 5 November 2026

These are **intermediate implementation captures**, not screenshots of another product or a pre-change baseline. They show a synthetic conversation and journal in an embedded desktop build with isolated data, in the light theme at 1280 × 840. No provider was connected. The capture is limited to the application window.

- [Activity before the commentary-label fix](intermediate-activity.png) shows the first recorded reasoning round and intermediate commentary. The commentary incorrectly presents a “Copy answer” action; the implementation was subsequently changed to label it “Update” and reserve that action for the final answer.
- [Activity after the fix](confirmed-activity.png) confirms the update label in the same synthetic fixture. The final answer retains its copy action.
- [Answer](intermediate-answer.png) shows the final response, code block and answer copy action. In this viewport, the composer and header leave a relatively short scrolling transcript, so a single frame cannot show the whole turn.
- [Saved request in the sidebar](saved-request-sidebar.png) and [recovery panel](saved-request.png) show a synthetic unsent request after reopening an isolated native build. The send action is disabled because the fixture has no configured model. The panel was scrolled into view for the capture.
- [Typed reasoning replay](typed-reasoning-replay.png) shows the first of two synthetic reasoning rounds in the embedded build. The fixture has typed item events and no legacy reasoning events; its displayed 1.2-second duration comes from the typed event. The capture is a completed replay in the light theme, not a live provider stream.
- [Interrupted tool draft](interrupted-tool-draft.png) shows a synthetic request that ended before dispatch, including its path preview. This is a light-theme replay, not a capture of an actively streaming tool request.

The images do not establish live stream ordering, timing accuracy, dark-theme or narrow-window behavior, provider integration, or user acceptance. Those require the scenarios and frame sequences in the conversation and harness plan. The complete intermediate fixture frames, including the worktree panel, are in `.local/qa/embedded-conversation-5/`.
