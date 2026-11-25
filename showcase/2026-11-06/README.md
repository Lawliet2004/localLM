# LocalLM YouTube Short

The latest finished video is `locallm-short-with-voiceover.mp4`: 60 seconds, vertical 1080 × 1920, 30 fps, H.264 video and stereo AAC audio. It includes real native desktop footage, large editorial titles, burned-in English captions, prominent synthetic English narration, and quiet original synthesized music. Its narration is explicitly marked as the default English audio track and measured at approximately -14 LUFS.

`locallm-voiceover-60s.mp3` is the standalone narration. `repair_voiceover.py` reproduces this audio revision and copies the original picture without reencoding. `voiceover-verification.json` records the latest audio checks. The earlier export remains available as `locallm-youtube-short-60s.mp4`.

`locallm-short-cover.jpg` is a portrait cover. `locallm-short-captions.srt` contains phrase-timed captions, with timing estimated from each measured narration segment. `youtube-publishing-copy.md` contains a suggested title and description. Nothing has been uploaded or published.

The demo ran `deepseek-ai/DeepSeek-V4.1-Flash-1017` through the existing River provider. It read the isolated demo workspace's README and CSV, requested approval to write `launch-summary.md`, created the report, read it back, and returned the correct totals: 745 signups, 110 paid conversions, and 14.8% overall conversion. The generated file was opened in the editor, staged through Git review, and read in the native terminal. No commit was made after generation.

The local GGUF model and runtime configuration are separate feature shots. This recording does not claim to demonstrate local GGUF inference, authenticated MCP accounts, or installed active skills. The sequence uses real footage with waiting time cut, close-ups, and held frames. No model responses or successful actions were fabricated.

The recording used an isolated X11 display, a disposable project, and separate application storage. Only the existing encrypted provider credential was temporarily copied; its text was never revealed or printed. The private recording profile was removed after capture. The user's application data, projects, existing chat history, and source edits were not used as demo content.

The standard `npm run build` encountered an existing TypeScript error in `src/components/Chat.tsx` involving `String.replaceAll` and the project's target library. Recording assets were instead bundled with `npx vite build`, and the embedded native application was built successfully with `cargo build --manifest-path src-tauri/Cargo.toml --bin locallm --features tauri/custom-protocol`. No application source changes were made for recording.

`capture-provenance.json` records the actual model run and file evidence. `video-verification.json` records the final media checks. Raw native footage is in `captures/`; the edit manifest and narration are in `assets/`. The editable rendering script is `render_short.py` and requires Python with Pillow and NumPy, plus FFmpeg. `create_narration.py` additionally requires edge-tts. The temporary virtual environment used for creation was `/tmp/locallm-short-venv`.

To rerender from the captured footage:

```sh
/tmp/locallm-short-venv/bin/python showcase/2026-11-06/render_short.py
```

The capture scripts are specific to this local development environment. Final deliverables contain no API keys.
