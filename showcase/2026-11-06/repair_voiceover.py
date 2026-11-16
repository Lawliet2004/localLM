"""Export an explicit foreground English narration track and remux the Short.

The original picture is copied without quality loss. A separate narration MP3
is included so the spoken audio can also be played independently.
"""
import json
import os
import subprocess
import wave
from pathlib import Path
os.environ['OPENBLAS_NUM_THREADS']='1'
import numpy as np

ROOT=Path(__file__).resolve().parent
A=ROOT/'assets'
segments=json.loads((A/'narration.json').read_text())
sr=48000
voice=np.zeros(sr*60,dtype=np.float32)
offset=0
for i,segment in enumerate(segments):
    rate=max(1,segment['voiceDuration']/(segment['duration']-.8))
    data=subprocess.check_output(['ffmpeg','-v','error','-i',str(A/segment['path']),
        '-af',f'atempo={rate}','-ar',str(sr),'-ac','1','-f','f32le','-'])
    signal=np.frombuffer(data,dtype='<f4').copy()
    active=signal[np.abs(signal)>.015]
    rms=float(np.sqrt(np.mean(active**2))) if len(active) else .1
    signal*=.17/max(rms,.01)
    start=round((offset+.35)*sr)
    count=min(len(signal),len(voice)-start)
    voice[start:start+count]+=signal[:count]
    offset+=segment['duration']

raw=A/'foreground-voice-raw.wav'
with wave.open(str(raw),'w') as w:
    w.setnchannels(1);w.setsampwidth(2);w.setframerate(sr)
    w.writeframes((np.clip(voice,-.98,.98)*32767).astype('<i2').tobytes())

master=A/'foreground-voice-60s.wav'
subprocess.run(['ffmpeg','-y','-v','error','-i',str(raw),'-af',
    'highpass=f=70,acompressor=threshold=0.1:ratio=2.5:attack=8:release=100:makeup=1.5,loudnorm=I=-14:TP=-1.5:LRA=7,apad,atrim=duration=60',
    '-ar','48000','-ac','2',str(master)],check=True)

mp3=ROOT/'locallm-voiceover-60s.mp3'
subprocess.run(['ffmpeg','-y','-v','error','-i',str(master),'-c:a','libmp3lame',
    '-b:a','192k','-id3v2_version','3','-metadata','title=LocalLM English Voiceover',str(mp3)],check=True)

mixed=A/'foreground-mix-60s.m4a'
# Keep the existing scored bed very low beneath the explicit narration stem.
subprocess.run(['ffmpeg','-y','-v','error','-i',str(master),'-i',str(A/'mastered-audio.wav'),
    '-filter_complex',
    '[0:a]volume=1.0[voice];[1:a]volume=0.035[bed];[voice][bed]amix=inputs=2:duration=first:normalize=0,loudnorm=I=-14:TP=-1.5:LRA=7,apad,atrim=duration=60[out]',
    '-map','[out]','-c:a','aac','-b:a','256k','-ar','48000','-ac','2',str(mixed)],check=True)

video=ROOT/'locallm-short-with-voiceover.mp4'
subprocess.run(['ffmpeg','-y','-v','error','-i',str(ROOT/'locallm-youtube-short-60s.mp4'),
    '-i',str(mixed),'-map','0:v:0','-map','1:a:0','-c','copy','-t','60',
    '-disposition:a:0','default','-metadata:s:a:0','language=eng',
    '-metadata:s:a:0','title=English narration',
    '-movflags','+faststart',str(video)],check=True)

print('Exported foreground narration:',video,flush=True)
print('Exported standalone voiceover:',mp3,flush=True)
