"""Render a 1080x1920, 30fps, 60-second native-application showcase.

Uses real native footage, editorial overlays, synthetic narration, and an
original procedurally composed music bed. Never fabricates application output.
"""
import json
import math
import os
import subprocess
import textwrap
import wave
from pathlib import Path

os.environ['OPENBLAS_NUM_THREADS'] = '1'
import numpy as np
from PIL import Image, ImageDraw, ImageFont, ImageFilter

ROOT = Path(__file__).resolve().parent
A = ROOT / 'assets'
C = ROOT / 'captures'
W, H, FPS = 1080, 1920, 30
CW, CH = 960, 800
WHITE = '#f3f3e9'
MINT = '#b9edba'
MUTED = '#9faea3'
BG = '#0b110f'
FONT = '/usr/share/fonts/truetype/space-grotesk/SpaceGrotesk-Regular.ttf'
BOLD = '/usr/share/fonts/truetype/space-grotesk/SpaceGrotesk-Bold.ttf'
MONO = '/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf'
NARRATION = json.loads((A / 'narration.json').read_text())
SCENES = [
    (0,4,'MEET YOUR NEW WORKSPACE',['Your AI.','Your workspace.'],'A desktop home for intelligence and action.'),
    (4,11,'01 / CHOOSE YOUR MODEL',['Choose your','intelligence.'],'Local GGUF + llama.cpp + API providers'),
    (11,18,'02 / CONNECT YOUR PROVIDER',['Your provider.','Your keys.'],'Your endpoint. Your model. Encrypted credentials.'),
    (18,32,'03 / WATCH IT WORK',['One request.','A real result.'],'Real DeepSeek inference via River · waiting time cut'),
    (32,40,'04 / KEEP WORK CONNECTED',['Stay in','one workspace.'],'Editor + Git review + native terminal'),
    (40,47,'05 / ADD MORE CAPABILITIES',['Extend what','AI can do.'],'MCP connectors + reusable skills'),
    (47,55,'06 / MAKE IT YOURS',['Stay in','control.'],'Permissions + inference + appearance'),
    (55,60,'LOCAL INTELLIGENCE. PRACTICAL WORK.',['From prompt','to practical work.'],'Meet LocalLM. Explore the source.'),
]

# Rectangles select actual application pixels. Controls or values are never
# recreated in graphics. Hold frames and close-ups are ordinary editorial cuts.
SHOTS = [
    (0,4,'answer',None,0,(400,280,1192,940),'NATIVE DESKTOP APPLICATION','Your model. Your files. Your workflow.'),
    (4,7.5,'models','model-files',1.95,(370,185,1162,845),'LOCAL MODEL SETUP','Compatible GGUF models · managed llama.cpp runtime'),
    (7.5,11,'models','runtime',4.9,(370,185,1162,845),'RUNTIME CONTROLS','Context window · GPU layers · CPU configuration'),
    (11,15,'api-key',None,0,(390,215,1146,845),'PROVIDER CONFIGURATION','Saved API key stays hidden on screen'),
    (15,18,'model-selection','model-selector',.9,(790,430,1198,1050),'MODEL SELECTION','Choose the model that fits the task'),
    (18,20.5,'prompt',None,0,(430,872,1170,1047),'01 / THE REQUEST','Read the metrics. Create a launch report.'),
    (20.5,23.5,'live-inference','inference-progress',15.9,(405,200,1197,860),'02 / READ THE WORKSPACE','Real file reads · README.md + stats.csv'),
    (23.5,27,'live-inference','approval',54.7,(280,230,1000,830),'03 / APPROVE THE WRITE','The app asks before creating the report'),
    (27,32,'live-inference','answer',102.0,(405,285,1197,945),'04 / INSPECT THE RESULT','745 signups · 110 paid · 14.8% conversion'),
    (32,34.7,'workspace-clean','editor-clean',None,(328,60,1228,810),'EDITOR','The actual model-created launch-summary.md'),
    (34.7,37.4,'workspace-clean','review-clean',None,(60,415,960,1090),'GIT REVIEW','Inspect and stage the generated file'),
    (37.4,40,'workspace-clean','terminal-clean',None,(60,102,1260,1100),'NATIVE TERMINAL','Read the generated report in a real shell'),
    (40,43.5,'extensions','connectors',1.1,(370,185,1162,845),'MCP CONNECTORS','Bring connected tools into the workspace'),
    (43.5,47,'extensions','skills',4.05,(370,185,1162,845),'REUSABLE SKILLS','Add instructions and workflows for repeatable work'),
    (47,49.7,'controls','permissions',25.9,(425,575,1145,1055),'APPROVAL MODES','Ask for approval · auto-approve reads · full access'),
    (49.7,52,'inference-settings',None,0,(370,185,1162,845),'INFERENCE SETTINGS','Reasoning · sampling · response length'),
    (52,55,'controls','appearance-light',22.8,(370,185,1162,845),'APPEARANCE','Light and dark themes · accents · reading controls'),
    (55,60,'ending',None,0,None,'EXPLORE THE PROJECT','github.com/Lawliet2004/localLM'),
]

def font(size, bold=False, mono=False):
    return ImageFont.truetype(MONO if mono else BOLD if bold else FONT, size)

FONTS = {(s,b,m):font(s,b,m) for s in [17,18,19,20,22,24,26,28,30,32,34,36,38,40,42,46,48,52,60,62,64,68,78,88,90,94,96,100,102,110] for b in [False,True] for m in [False,True]}
def f(size,b=False,m=False): return FONTS[(size,b,m)]

def tracked(d,xy,s,fill=MUTED,size=18,spacing=2):
    x,y=xy
    for ch in s:
        d.text((x,y),ch,font=f(size,False,True),fill=fill)
        x+=d.textlength(ch,font=f(size,False,True))+spacing

def brand(d,x,y,size=48):
    d.rounded_rectangle((x,y,x+size,y+size),radius=int(size*.23),fill='#253d30')
    k=size/100
    d.rectangle((x+28*k,y+25*k,x+39*k,y+76*k),fill=WHITE)
    d.rectangle((x+28*k,y+66*k,x+71*k,y+76*k),fill=WHITE)
    d.ellipse((x+59*k,y+21*k,x+77*k,y+39*k),fill=MINT)

def make_background():
    yy,xx=np.mgrid[0:H,0:W]
    glow=np.exp(-(((xx-1020)/780)**2+((yy-530)/1050)**2))
    glow2=np.exp(-(((xx-130)/550)**2+((yy-1500)/550)**2))
    pixels=np.stack([10+7*glow+2*glow2,16+15*glow+6*glow2,13+9*glow+3*glow2],axis=-1).astype(np.uint8)
    im=Image.fromarray(pixels)
    d=ImageDraw.Draw(im)
    for x in range(60,W,80):d.line((x,185,x,1760),fill='#18201b',width=1)
    for y in range(210,1760,80):d.line((60,y,1020,y),fill='#18201b',width=1)
    # Quiet grid ends at the interface card, keeping the typography clean.
    d.rectangle((0,0,W,190),fill=BG)
    return im

BACKGROUND=make_background()
MASK=Image.new('L',(CW,CH),0)
ImageDraw.Draw(MASK).rounded_rectangle((0,0,CW-1,CH-1),radius=21,fill=255)

def base_scene(i):
    start,end,kicker,lines,sub=SCENES[i]
    im=BACKGROUND.copy();d=ImageDraw.Draw(im)
    brand(d,60,66)
    d.text((124,67),'localLM',font=f(38,True),fill=WHITE)
    tracked(d,(770,81),'60-SECOND TOUR',size=17,spacing=1.1)
    d.line((60,153,1020,153),fill='#344238',width=1)
    # The chapter marker and title are editorial graphics around the real app.
    d.rounded_rectangle((60,206,1020,247),radius=9,fill='#18251c')
    tracked(d,(76,216),kicker,fill=MINT,size=17,spacing=1.1)
    size=94 if i in (0,7) else 100
    for row,line in enumerate(lines):
        d.text((54,279+row*108),line,font=f(size,True),fill=WHITE if row==0 else MINT)
    d.text((62,506),sub,font=f(26),fill=MUTED)
    if i!=7:
        shadow=Image.new('RGBA',(W,H))
        sd=ImageDraw.Draw(shadow);sd.rounded_rectangle((46,576,1034,1450),radius=32,fill=(0,0,0,160))
        shadow=shadow.filter(ImageFilter.GaussianBlur(18));im=Image.alpha_composite(im.convert('RGBA'),shadow).convert('RGB');d=ImageDraw.Draw(im)
        d.rounded_rectangle((58,575,1022,1432),radius=25,fill='#17221b',outline='#4a6050',width=2)
        for x in (82,103,124):d.ellipse((x,594,x+9,603),fill='#55705b')
    d.line((60,1770,1020,1770),fill='#304036',width=2)
    tracked(d,(60,1820),'LOCAL MODELS / CLOUD / REAL TOOLS',size=17,spacing=.7)
    d.text((933,1813),f'{i+1:02d}/08',font=f(22,False,True),fill=MUTED)
    return im

BASES=[base_scene(i) for i in range(8)]

def scene_index(t): return next(i for i,s in enumerate(SCENES) if s[0]<=t<s[1])

def caption_lines(text,max_width=880):
    d=ImageDraw.Draw(Image.new('RGB',(1,1)))
    lines=[];line=''
    for word in text.split():
        candidate=(line+' '+word).strip()
        if d.textlength(candidate,font=f(40,True))>max_width and line:lines.append(line);line=word
        else:line=candidate
    if line:lines.append(line)
    return lines

CAPTIONS=[]
voice_lengths=[]
offset=0
for i,n in enumerate(NARRATION):
    available=n['duration']-.8
    ratio=max(1,n['voiceDuration']/available)
    length=n['voiceDuration']/ratio
    voice_lengths.append((ratio,length))
    words=n['text'].replace('Local L M','LocalLM').replace('A P I','API').replace('A I','AI').replace('M C P','MCP').split()
    chunks=[]
    while words:
        count=min(7,len(words))
        # Avoid a short orphan at the end of a sentence.
        if len(words)-count<3 and len(words)>count: count=len(words)//2
        chunk=' '.join(words[:count]);words=words[count:];chunks.append(chunk)
    weights=[len(c) for c in chunks];total=sum(weights);cursor=offset+.35
    for c,weight in zip(chunks,weights):
        dt=length*weight/total
        CAPTIONS.append((cursor,cursor+dt,c))
        cursor+=dt
    offset+=n['duration']

def srt_time(s):
    ms=int(round(s*1000));return f'{ms//3600000:02d}:{ms//60000%60:02d}:{ms//1000%60:02d},{ms%1000:03d}'

(ROOT/'locallm-short-captions.srt').write_text('\n\n'.join(f'{i+1}\n{srt_time(a)} --> {srt_time(b)}\n{c}' for i,(a,b,c) in enumerate(CAPTIONS))+'\n')

def prepare_audio():
    sr=48000;n=sr*60
    voice=np.zeros(n,dtype=np.float32)
    offset=0
    for i,item in enumerate(NARRATION):
        ratio,length=voice_lengths[i]
        raw=subprocess.check_output(['ffmpeg','-v','error','-i',str(A/item['path']),'-af',f'atempo={ratio}',
            '-ar',str(sr),'-ac','1','-f','f32le','-'])
        sig=np.frombuffer(raw,dtype='<f4').copy()
        active=sig[np.abs(sig)>.015]
        rms=np.sqrt(np.mean(active**2)) if len(active) else .1
        sig*=.17/max(rms,.01)
        start=int((offset+.35)*sr);limit=min(len(sig),n-start)
        voice[start:start+limit]+=sig[:limit]
        offset+=item['duration']
    music=np.zeros(n,dtype=np.float32);rng=np.random.default_rng(2004)
    beat=60/108
    chords=[(110,164.8138,220,261.6256),(87.3071,130.8128,174.6141,220),
            (130.8128,164.8138,195.9977,246.9417),(97.9989,146.8324,195.9977,293.6648)]
    def add(at,sig):
        start=int(at*sr)
        if start>=n:return
        stop=min(n,start+len(sig));music[start:stop]+=sig[:stop-start]
    # Original four-chord pulse; no third-party music or samples are used.
    for bar in range(28):
        at=bar*4*beat;chord=chords[bar%4]
        t=np.arange(int(4.5*beat*sr))/sr
        env=np.minimum(t/.16,1)*np.exp(-t/1.7)
        pad=sum(np.sin(2*np.pi*q*t+.07*np.sin(2*np.pi*.23*t)) for q in chord)/4
        add(at,pad*env*.055)
        for j in range(8):
            tp=np.arange(int(.45*sr))/sr;q=chord[(j*3)%4]*4
            pluck=(np.sin(2*np.pi*q*tp)+.18*np.sin(4*np.pi*q*tp))*np.exp(-tp*14)
            add(at+j*beat/2,pluck*.037)
        for j in range(4):
            tk=np.arange(int(.20*sr))/sr
            kick=np.sin(2*np.pi*(47*tk+1.3*(1-np.exp(-tk*35))))*np.exp(-tk*23)
            add(at+j*beat,kick*.055)
            th=np.arange(int(.045*sr))/sr
            noise=rng.normal(0,1,len(th));noise=np.r_[0,np.diff(noise)]
            add(at+(j+.5)*beat,noise*np.exp(-th*105)*.006)
    # Soft transition accents mark each editorial chapter.
    for start in [4,11,18,32,40,47,55]:
        ts=np.arange(int(.22*sr))/sr
        whoosh=rng.normal(0,1,len(ts));whoosh=np.convolve(whoosh,np.ones(8)/8,mode='same')
        add(start-.13,whoosh*np.sin(np.pi*ts/.22)**2*.025)
    t=np.arange(n)/sr
    music*=np.minimum(t/1.5,1)*np.minimum((60-t)/2.5,1)
    # Smooth ducking follows real narration amplitude.
    envelope=np.convolve(np.abs(voice)[::480],np.ones(21)/21,mode='same')
    duck=np.interp(np.arange(n),np.arange(len(envelope))*480,np.clip(1-envelope*4,.38,1))
    left=voice+music*duck;right=voice+np.roll(music,360)*duck*.94
    stereo=np.stack([left,right],axis=1)
    peak=np.max(np.abs(stereo))
    if peak>.94:stereo*=.94/peak
    with wave.open(str(A/'original-mix.wav'),'w') as out:
        out.setnchannels(2);out.setsampwidth(2);out.setframerate(sr)
        out.writeframes((stereo*32767).astype('<i2').tobytes())
    subprocess.run(['ffmpeg','-y','-v','error','-i',str(A/'original-mix.wav'),'-af',
        'loudnorm=I=-16:TP=-1.5:LRA=9','-ar','48000',str(A/'mastered-audio.wav')],check=True)

def resolve_offsets():
    events=json.loads((A/'capture-events.json').read_text())
    take=None;offsets={}
    for e in events:
        if e['op']=='record':take=e['name']
        if e['op']=='snap':offsets[(take,e['name'])]=e['recordOffset']
    return offsets

class Source:
    def __init__(self,shot,offsets):
        a,b,name,snapshot,offset,rect,label,note=shot
        self.process=None;self.still=None;self.last=None;self.special=name in ('prompt','ending')
        if name=='ending':return
        if snapshot is None:
            self.still=Image.open(C/(name+'.png')).convert('RGB').crop(rect)
            return
        if offset is None:offset=max(0,offsets[(name,snapshot)])
        x,y,r,bot=rect;cw,ch=r-x,bot-y
        # The model menu is a tall popover, so preserve it with letterboxing.
        if snapshot=='model-selector':
            vf=f'crop={cw}:{ch}:{x}:{y},scale={CW}:{CH}:force_original_aspect_ratio=decrease,pad={CW}:{CH}:(ow-iw)/2:(oh-ih)/2:color=0x1b261f,fps=30'
        else:vf=f'crop={cw}:{ch}:{x}:{y},scale={CW}:{CH}:force_original_aspect_ratio=increase,crop={CW}:{CH},fps=30'
        self.process=subprocess.Popen(['ffmpeg','-v','error','-threads','2','-ss',str(offset),'-i',str(C/(name+'.mp4')),
            '-an','-t',str(b-a+.1),'-vf',vf,'-pix_fmt','rgb24','-f','rawvideo','-'],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
    def frame(self):
        if self.still:return self.still
        if self.process:
            needed=CW*CH*3;buf=bytearray()
            while len(buf)<needed:
                part=self.process.stdout.read(needed-len(buf))
                if not part:break
                buf.extend(part)
            if len(buf)==needed:self.last=Image.frombytes('RGB',(CW,CH),bytes(buf))
            if self.last:return self.last
            raise RuntimeError('Native footage could not be decoded')
        return None
    def close(self):
        if self.process:
            self.process.stdout.close();self.process.terminate();self.process.wait()

def centered(d,text,y,size=40,fill=WHITE,bold=True):
    ft=f(size,bold);width=d.textlength(text,font=ft);d.text(((W-width)/2,y),text,font=ft,fill=fill)

def render_frame(t,shot,source,chapter):
    im=BASES[chapter].copy();d=ImageDraw.Draw(im)
    a,b,name,snapshot,offset,rect,label,note=shot
    local=t-a;progress=(t-SCENES[chapter][0])/(SCENES[chapter][1]-SCENES[chapter][0])
    if name=='ending':
        d.rounded_rectangle((60,600,1020,1450),radius=28,fill='#18271d',outline='#4d6b52',width=2)
        brand(d,448,680,184)
        centered(d,'localLM',910,88)
        centered(d,'A desktop AI workspace.',1022,34,fill=MUTED,bold=False)
        for j,item in enumerate(['LOCAL MODELS','API PROVIDERS','REAL TOOLS']):
            y=1122+j*74
            d.ellipse((235,y+12,247,y+24),fill=MINT)
            d.text((271,y),item,font=f(28,True),fill=WHITE)
        d.rounded_rectangle((124,1362,956,1426),radius=13,fill=MINT)
        centered(d,'github.com/Lawliet2004/localLM',1378,28,fill='#142119')
    elif name=='prompt':
        d.rounded_rectangle((60,630,1020,1430),radius=21,fill='#202d24')
        d.text((108,700),'READ THE METRICS.',font=f(48,True),fill=WHITE)
        d.text((108,761),'WRITE THE REPORT.',font=f(48,True),fill=MINT)
        tracked(d,(110,873),'ACTUAL REQUEST IN THE APP',size=18,spacing=1)
        shot_image=source.frame();shot_image=shot_image.resize((900,213),Image.Resampling.LANCZOS)
        im.paste(shot_image,(90,943))
        d=ImageDraw.Draw(im)
        for i,(text,width) in enumerate([('README.md',240),('stats.csv',210),('launch-summary.md',350)]):
            x=[106,369,604][i]
            d.rounded_rectangle((x,1235,x+width,1290),radius=10,fill='#304736',outline='#54715a')
            d.text((x+18,1250),text,font=f(22,False,True),fill=WHITE)
    else:
        content=source.frame()
        if content:
            if content.size!=(CW,CH):
                # Gentle camera motion is applied only to a truthful still.
                zoom=1.0+.018*min(1,local/max(.1,b-a))
                content=content.resize((round(CW*zoom),round(CH*zoom)),Image.Resampling.LANCZOS)
                left=(content.width-CW)//2;top=(content.height-CH)//2
                content=content.crop((left,top,left+CW,top+CH))
            im.paste(content,(60,630),MASK)
        d=ImageDraw.Draw(im)
    if name!='ending':
        ft=f(18,False,True);lw=d.textlength(label,font=ft)
        d.text(((W-lw)/2,588),label,font=ft,fill='#c4d6c6')
    # A narrow moving line gives the frame an intentional pace without hiding UI.
    d.line((60,1457,60+960*min(1,max(0,progress)),1457),fill=MINT,width=3)
    lines=textwrap.wrap(note,66)
    for i,line in enumerate(lines[:2]):centered(d,line,1485+i*30,24,fill=MUTED,bold=False)
    for ca,cb,text in CAPTIONS:
        if ca<=t<cb:
            lines=caption_lines(text)
            y=1590 if len(lines)>1 else 1615
            for i,line in enumerate(lines[:2]):centered(d,line,y+i*52,40)
            break
    d.rounded_rectangle((60,1767,60+960*t/60,1773),radius=3,fill=MINT)
    # Discrete chapter ticks make the complete one-minute tour easy to follow.
    for s in SCENES[1:]:
        x=60+960*s[0]/60;d.line((x,1764,x,1778),fill='#829a85',width=2)
    return im

def main():
    prepare_audio();offsets=resolve_offsets()
    (A/'edit-manifest.json').write_text(json.dumps({'scenes':SCENES,'shots':SHOTS,'captionTiming':'Phrase timing estimated from each measured narration segment.','duration':60,'fps':30},indent=2))
    out=ROOT/'locallm-youtube-short-60s.mp4'
    writer=subprocess.Popen(['ffmpeg','-y','-loglevel','error','-f','rawvideo','-pixel_format','rgb24','-video_size',f'{W}x{H}',
        '-framerate',str(FPS),'-i','-','-i',str(A/'mastered-audio.wav'),'-map','0:v','-map','1:a',
        '-c:v','libx264','-threads','6','-preset','fast','-crf','18','-pix_fmt','yuv420p',
        '-color_primaries','bt709','-color_trc','bt709','-colorspace','bt709',
        '-c:a','aac','-b:a','192k','-ar','48000','-t','60','-movflags','+faststart',str(out)],stdin=subprocess.PIPE)
    previews=[]
    try:
        for si,shot in enumerate(SHOTS):
            start,end=shot[:2];source=Source(shot,offsets)
            try:
                for fi in range(round(start*FPS),round(end*FPS)):
                    t=fi/FPS;chapter=scene_index(t)
                    frame=render_frame(t,shot,source,chapter)
                    if fi==round(start*FPS)+min(24,round((end-start)*FPS)-1):
                        frame.save(A/f'preview-{si+1:02d}.jpg',quality=94)
                        previews.append(frame.copy())
                    writer.stdin.write(frame.tobytes())
            finally:source.close()
            print(f'Edited {end:04.1f} / 60.0 seconds: {shot[-2]}',flush=True)
    finally:writer.stdin.close()
    if writer.wait()!=0:raise RuntimeError('Video encoder failed')
    sheet=Image.new('RGB',(1080,480*math.ceil(len(previews)/4)),BG)
    for i,im in enumerate(previews):sheet.paste(im.resize((270,480),Image.Resampling.LANCZOS),((i%4)*270,(i//4)*480))
    sheet.save(ROOT/'edit-contact-sheet.jpg',quality=92)
    cover=BASES[0].copy();d=ImageDraw.Draw(cover)
    # A dedicated portrait cover uses the final result from the real model run.
    screenshot=Image.open(C/'answer.png').crop((400,280,1192,940)).resize((CW,CH),Image.Resampling.LANCZOS)
    cover.paste(screenshot,(60,630),MASK);d=ImageDraw.Draw(cover)
    centered(d,'AI THAT WORKS WITH YOUR FILES.',1530,38,fill=MINT)
    centered(d,'LOCAL MODELS. APIS. REAL TOOLS.',1600,28,fill=WHITE)
    cover.save(ROOT/'locallm-short-cover.jpg',quality=97)
    print('Finished:',out,flush=True)

if __name__=='__main__':main()
