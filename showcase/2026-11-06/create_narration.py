"""Create original synthetic narration for an exactly sixty-second edit."""
import asyncio
import json
import subprocess
from pathlib import Path
import edge_tts

ROOT = Path(__file__).resolve().parent
ASSETS = ROOT / 'assets'
SCENES = [
    (4, 'What if your A I could actually work with your files?'),
    (7, 'Meet Local L M. A desktop workspace for local models and your favorite A P I providers.'),
    (7, 'Connect your endpoint, keep your A P I key encrypted on your device, and choose a model.'),
    (14, 'Now watch a real request. The model reads these launch metrics, creates a report, and returns the result, with every tool action visible.'),
    (8, 'Open your files, review the changes, and work in the terminal, without leaving the conversation.'),
    (7, 'Add M C P connectors and reusable skills to bring more tools into your workflow.'),
    (8, 'Choose approval controls, tune inference and appearance, and keep your projects and conversations organized.'),
    (5, 'Local L M. From a prompt to practical work. Explore the source.'),
]

async def main():
    metadata = []
    for i, (duration, line) in enumerate(SCENES):
        path = ASSETS / f'voice-{i+1:02d}.mp3'
        if not path.exists():
            await edge_tts.Communicate(line, 'en-US-AndrewMultilingualNeural', rate='+6%').save(str(path))
        length = float(subprocess.check_output(['ffprobe','-v','error','-show_entries','format=duration',
            '-of','default=nw=1:nk=1',str(path)]))
        metadata.append({'duration':duration, 'voiceDuration':length, 'text':line, 'path':path.name})
        print(f'Voice {i+1}: {length:.2f}s / {duration}s', flush=True)
    (ASSETS/'narration.json').write_text(json.dumps(metadata,indent=2))

if __name__ == '__main__': asyncio.run(main())
