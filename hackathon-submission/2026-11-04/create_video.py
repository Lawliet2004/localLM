"""Rebuild the narrated overview; requires Pillow, edge-tts, ffmpeg, ffprobe.

Uses explanatory graphics, never simulated application output.
"""
import asyncio
import json
import subprocess
import textwrap
from pathlib import Path

import edge_tts
from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent
ASSETS = ROOT / "video-assets"
ASSETS.mkdir(exist_ok=True)
FONT = "/usr/share/fonts/truetype/space-grotesk/SpaceGrotesk-Regular.ttf"
BOLD = "/usr/share/fonts/truetype/space-grotesk/SpaceGrotesk-Bold.ttf"
SCENES = [
    (20, "Your AI workspace.\nOn your terms.", ["LocalLM", "Powered by Meta's Muse Spark 1.3"], "AI can explain a task. But getting it done often means moving between a chatbot, files, a terminal, and Git. Local L M brings those workflows into one desktop workspace, connecting an assistant to practical work with visible actions and user control."),
    (20, "Less context switching.\nMore connected work.", ["Conversation", "Files and code", "Tools and review"], "The problem is the gap between a conversation and a finished result. Copying code and context between separate applications takes effort, and makes changes harder to track. Local L M brings the conversation, editor, terminal, and Git review together."),
    (25, "Meta intelligence.\nA desktop harness.", ["Muse Spark 1.3: hosted assistant", "LocalLM: context, permissions, tools", "Results return to the conversation"], "For this hackathon submission, Meta's Muse Spark one point three powers the assistant inside Local L M. The model supplies reasoning and responses. Local L M supplies selected workspace context and enabled tools, checks permissions, executes authorized actions, and returns the results so the assistant can continue."),
    (25, "From a request\nto a reviewable change.", ["Inspect selected project files", "Make an authorized edit", "Review the diff and check the result"], "A coding workflow starts with a request to inspect a project. The assistant can read selected files, explain the code, and propose an improvement. With the appropriate tools and permission, it can make an edit. The user can then inspect the diff and run a relevant check in the same workspace."),
    (25, "You choose\nthe level of access.", ["Ask for approval", "Auto-approve reads", "Full access, with permission rules"], "Local L M offers approval modes and permission rules. A user can require approval, automatically permit supported workspace reads, or select full access for enabled tools. Tool decisions and results stay in the activity history. Local execution uses the user's account permissions, so these controls matter."),
    (25, "Build beyond\na single conversation.", ["MCP connectors and reusable skills", "Saved conversations and tool activity", "Inspectable output artifacts"], "M C P connectors and reusable skills extend the assistant's workflow. Saved conversations preserve context and tool activity. Larger outputs can be captured as artifacts for inspection, instead of overwhelming the conversation. These features give the model a practical environment for working across a task."),
    (20, "Choose your\ninference path.", ["Hosted: Muse Spark 1.3 for this entry", "Local: compatible GGUF via llama.cpp", "Rust + Tauri 2 + React + TypeScript"], "Local L M supports configured hosted providers and compatible local G G U F models through llama dot C P P. Muse Spark is the hosted model used for this submission. Local inference is a separate option. The desktop application is built with Rust, Tauri, React, and TypeScript."),
    (20, "Model intelligence.\nPractical user control.", ["LocalLM", "github.com/Lawliet2004/localLM"], "Local L M connects Meta's model intelligence to files, code, and tools in a desktop workspace. The goal is straightforward: help users move from a request to an inspectable result, while retaining control over the actions taken. Explore the project on GitHub. Thank you."),
]


def run(args):
    return subprocess.run(args, check=True, capture_output=True, text=True).stdout


def draw_slide(i, title, bullets):
    im = Image.new("RGB", (1280, 720), "#0a1421")
    d = ImageDraw.Draw(im)
    accent = "#78e1bc"
    regular = lambda n: ImageFont.truetype(FONT, n)
    bold = lambda n: ImageFont.truetype(BOLD, n)
    d.rectangle((0, 0, 11, 720), fill=accent)
    d.text((64, 36), "LocalLM / META AI DEVELOPER HACKATHON", font=regular(20), fill=accent)
    d.text((1050, 36), f"{i+1:02d} / 08", font=regular(20), fill="#b5c5d5")
    d.multiline_text((64, 111), title, font=bold(57), fill="#f1f5f9", spacing=5)
    for j, item in enumerate(bullets):
        y = 300 + j * 55
        d.ellipse((68, y+10, 77, y+19), fill=accent)
        d.text((96, y), item, font=regular(27), fill="#c5d4e2")
    # Explicit conceptual flow; no invented app screens or successful tool output.
    labels = ["Your request", "Model + context", "Permitted tools", "Review result"]
    for j, label in enumerate(labels):
        x = 64 + j * 292
        d.rounded_rectangle((x, 500, x+260, 567), radius=12, fill="#142639", outline="#28465c", width=2)
        d.text((x+18, 521), label, font=regular(21), fill=accent if j == i % 4 else "#c5d4e2")
        if j < 3:
            d.text((x+270, 519), "›", font=regular(26), fill=accent)
    d.line((64, 614, 1216, 614), fill="#28465c", width=1)
    d.text((64, 642), "NARRATED OVERVIEW • EXPLANATORY GRAPHICS • NO LIVE APP FOOTAGE", font=regular(17), fill="#9bb0c3")
    d.rectangle((0, 710, 1280*(i+1)//8, 720), fill=accent)
    p = ASSETS / f"scene-{i+1:02d}.png"
    im.save(p)
    return p


async def main():
    transcript = []
    offset = 0
    for i, (duration, title, bullets, narration) in enumerate(SCENES):
        png = draw_slide(i, title, bullets)
        audio = ASSETS / f"scene-{i+1:02d}.mp3"
        if not audio.exists():
            await edge_tts.Communicate(narration, "en-US-GuyNeural", rate="+3%").save(str(audio))
        audio_len = float(run(["ffprobe", "-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1", str(audio)]).strip())
        if audio_len > duration-0.4:
            raise RuntimeError(f"Scene {i+1} narration is too long: {audio_len}")
        target = ASSETS / f"scene-{i+1:02d}.mp4"
        run(["ffmpeg", "-y", "-loglevel", "error", "-loop", "1", "-framerate", "12", "-i", str(png), "-i", str(audio), "-af", "apad", "-t", str(duration), "-c:v", "libx264", "-preset", "fast", "-crf", "20", "-tune", "stillimage", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "160k", "-ar", "48000", str(target)])
        transcript.append(f"{offset//60}:{offset%60:02d} — {title.replace(chr(10), ' ')}\n\n{narration}\n")
        offset += duration
        print(f"Scene {i+1} ready; voice {audio_len:.1f}s / {duration}s", flush=True)
    concat = ASSETS / "concat.txt"
    concat.write_text("".join(f"file 'scene-{i+1:02d}.mp4'\n" for i in range(8)))
    run(["ffmpeg", "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", str(concat), "-c", "copy", "-movflags", "+faststart", str(ROOT / "locallm-overview.mp4")])
    (ROOT / "video-transcript.md").write_text("# Narrated overview transcript\n\nSynthetic narration. Explanatory graphics; no live app recording.\n\n" + "\n".join(transcript))
    sheet = Image.new("RGB", (1280, 720))
    for i in range(8):
        with Image.open(ASSETS / f"scene-{i+1:02d}.png") as slide:
            sheet.paste(slide.resize((320, 360)), ((i%4)*320, (i//4)*360))
    sheet.save(ASSETS / "contact-sheet.png")
    info = json.loads(run(["ffprobe", "-v", "error", "-show_format", "-show_streams", "-of", "json", str(ROOT / "locallm-overview.mp4")]))
    (ROOT / "video-verification.json").write_text(json.dumps(info, indent=2))
    print(f"Finished: {info['format']['duration']} seconds", flush=True)


if __name__ == "__main__":
    asyncio.run(main())
