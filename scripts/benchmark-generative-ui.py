#!/usr/bin/env python3
"""Offline GGUF benchmark using a separate CPU llama.cpp process and read-only settings.
Never changes the user's runtime, database, workspace or configured memory limit.
"""
import argparse,hashlib,json,os,platform,secrets,signal,socket,sqlite3,subprocess,sys,time,urllib.request
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--preferences-db',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
p.add_argument('--repetitions',type=int,default=3)
p.add_argument('--cases',help='Comma-separated case names; default is the full matrix')
p.add_argument('--cache-policy',choices=['disabled','warm'],default='disabled')
a=p.parse_args();
if not 2<=a.repetitions<=20:p.error('repetitions must be 2..20')
a.output.mkdir(parents=True,exist_ok=True)
with sqlite3.connect(a.preferences_db.resolve().as_uri()+'?mode=ro',uri=True) as db:
 settings={k:json.loads(v) for k,v in db.execute("SELECT key,value FROM settings WHERE key IN ('preferences','runtime')")}
prefs,config=settings['preferences'],settings['runtime'];key=secrets.token_urlsafe(32)
with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
url=f'http://127.0.0.1:{port}'
env={k:v for k,v in os.environ.items() if not k.startswith('LLAMA_')};env['LLAMA_API_KEY']=key
args=[prefs['runtimePath'],'--model',prefs['modelPath'],'--ctx-size',str(min(config['contextLength'],8192)),'--threads',str(config['cpuThreads']),'--n-gpu-layers','0','--no-kv-offload','--batch-size','128','--ubatch-size','128','--parallel','1','--host','127.0.0.1','--port',str(port),'--jinja','--no-webui']
helptext=subprocess.run([prefs['runtimePath'],'--help'],capture_output=True,text=True,env=env,timeout=20).stdout
if '--no-agent' in helptext:args+=['--no-agent']
if '--fit' in helptext:args+=['--fit','off']
def request(path,data=None,timeout=180):
 return urllib.request.urlopen(urllib.request.Request(url+path,data=json.dumps(data).encode() if data is not None else None,headers={'Authorization':'Bearer '+key,'Content-Type':'application/json'}),timeout=timeout)
def active_compilers():
 result=[]
 for entry in Path('/proc').iterdir():
  if not entry.name.isdigit():continue
  try:
   name=(entry/'comm').read_text().strip()
   if name in {'rustc','clippy-driver','rust-lld','ld.lld','cc1','cc1plus'}:result.append(name)
  except OSError:pass
 return result
def wait_for_compilers():
 started=time.monotonic()
 while active_compilers():
  if time.monotonic()-started>180:break
  time.sleep(.3)
 return (time.monotonic()-started)*1000
def rss(pid):
 try:return int(next(line.split()[1] for line in Path(f'/proc/{pid}/status').read_text().splitlines() if line.startswith('VmHWM:')))
 except (OSError,StopIteration):return None
cases=[
 ('markdown',None,'Explain DNS in two simple sentences.'),
 ('core','core','Show the TCP handshake. Facts: client sends SYN to server; server sends SYN-ACK to client; client sends ACK to server. Use Diagram with only these facts.'),
 ('chart-bound','chart','Plot this authoritative supplied table as a bar chart. Use SourceChart with tableIndex 0.\n| Month | Units Sold |\n| --- | --- |\n| Jan | 10 |\n| Feb | 15 |\n| Mar | 25 |'),
 ('chart','chart','Plot these supplied sales: Jan 10, Feb 15, Mar 25. Use Chart with kind bar and exactly these points.'),
 ('data','data','Plot these supplied sales: Jan 10, Feb 15, Mar 25. Use Chart with kind bar and exactly these points.'),
 ('education','education','Teach binary search interactively using InteractiveExample with id search, kind binary-search, values [2,4,6,8,10], target 8.'),
 ('quiz','education','Make one SQL joins Quiz with id joins. Question: Which join preserves unmatched left rows? Options Inner, Left. Correct zero-based index 1. Explanation: LEFT JOIN preserves every left row.'),
 ('actual-process-result','coding','Present the supplied code and actual stdout verbatim using FilePreview for example.py and Text for stdout.'),
]
if a.cases:
 requested=set(a.cases.split(','));unknown=requested-{name for name,_,_ in cases}
 if unknown:p.error('Unknown cases: '+','.join(sorted(unknown)))
 cases=[case for case in cases if case[0] in requested]
def sha(path):
 h=hashlib.sha256()
 with Path(path).open('rb') as f:
  for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
 return h.hexdigest()
report={'startedAtUtc':time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime()),'model':Path(prefs['modelPath']).name,'modelSha256':sha(prefs['modelPath']),'runtimeSha256':sha(prefs['runtimePath']),'sourceSha256':{str(path):sha(path) for path in sorted(Path('src/genui').glob('*.ts*'))},'hardware':{'platform':platform.platform(),'cpu':next((line.split(':',1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),''),'logicalCpus':os.cpu_count()},'cpuOnly':True,'contextLength':min(config['contextLength'],8192),'maxOutputTokens':min(prefs['maxTokens'],512),'repetitions':a.repetitions,'cachePolicy':a.cache_policy,'loadPolicy':'Wait for observed Rust/link compiler idle before each request; before/after compiler observations recorded. Desktop background activity is not controlled; OS page cache not flushed.','cases':[],'limitations':['Isolated CPU process loaded once; first pass OS/model warmth is not controlled. cache_prompt is explicit and case order rotates.','TTFT is model transport, not first paint. Native interaction/memory are measured separately.','Actual Python process execution is fixed benchmark orchestration, not a model-selected permission-controlled harness call.','Direct program-generation experiment; production tool-free second-pass inference is separately measured in native acceptance. No remote provider or GPU inference.']}

process=None
try:
 with (a.output/'runtime.log').open('w') as log:
  process=subprocess.Popen(args,env=env,stdin=subprocess.DEVNULL,stdout=log,stderr=log,start_new_session=True)
  start=time.monotonic()
  while time.monotonic()-start<180:
   if process.poll() is not None:raise RuntimeError('Runtime exited; inspect runtime.log')
   try:
    with request('/health',timeout=2) as response:
     if response.status==200:break
   except Exception:time.sleep(.3)
  else:raise RuntimeError('Runtime startup timeout')
  report['loadSeconds']=time.monotonic()-start
  for sample in range(a.repetitions):
   for name,tier,question in cases[sample%len(cases):]+cases[:sample%len(cases)]:
    build_wait=wait_for_compilers()
    compilers_before=active_compilers()
    tool=None
    if name=='actual-process-result':
     tool_start=time.monotonic();tool_process=subprocess.run([sys.executable,'-c','print(1+1)'],capture_output=True,text=True,check=True,timeout=10)
     tool={'command':[sys.executable,'-c','print(1+1)'],'exitCode':tool_process.returncode,'stdout':tool_process.stdout,'durationMs':(time.monotonic()-tool_start)*1000}
     question+=' Path: example.py. Code: print(1+1). Actual stdout: '+tool_process.stdout
    prompt=('' if tier is None else Path(f'src/genui/generated/{tier}.txt').read_text()+'\nOutput only one complete OpenUI program, no fences or prose.\n')+question
    payload={'messages':[{'role':'user','content':prompt}],'max_tokens':report['maxOutputTokens'],'temperature':0,'stream':True,'stream_options':{'include_usage':True},'chat_template_kwargs':{'enable_thinking':False},'cache_prompt':a.cache_policy=='warm'}
    start=time.monotonic();first=None;content='';finish=None;usage=None;timings=None;tokens=None;failure=None
    try:
     with request('/tokenize',{'content':prompt,'add_special':False}) as response:tokens=len(json.load(response)['tokens'])
     start=time.monotonic()
     with request('/v1/chat/completions',payload,timeout=240) as response:
      for line in response:
       if not line.startswith(b'data:'):continue
       data=line[5:].strip()
       if data==b'[DONE]':break
       event=json.loads(data);choices=event.get('choices',[])
       if choices:
        delta=choices[0].get('delta',{}).get('content','') or ''
        if delta and first is None:first=time.monotonic()-start
        content+=delta;finish=choices[0].get('finish_reason') or finish
       usage=event.get('usage') or usage;timings=event.get('timings') or timings
    except Exception as error:
     failure=type(error).__name__+': '+str(error)[:300]
    item={'name':name,'sample':sample,'failure':failure,'compilerWaitMs':build_wait,'compilersBefore':compilers_before,'compilersAfter':active_compilers(),'actualProcess':tool,'profile':tier,'authoritativeInput':question,'rawContentTokens':tokens,'promptTokens':tokens,'timeToFirstTokenMs':None if first is None else first*1000,'totalMs':(time.monotonic()-start)*1000,'finishReason':finish,'source':content,'usage':usage,'timings':timings,'runtimePeakRssKiB':rss(process.pid)}
    report['cases'].append(item);(a.output/'results.json').write_text(json.dumps(report,indent=2));print(f'{name}: {item["totalMs"]:.0f} ms, {finish}, {len(content)} chars',flush=True)
finally:
 report['finishedAtUtc']=time.strftime('%Y-%m-%dT%H:%M:%SZ',time.gmtime())
 (a.output/'results.json').write_text(json.dumps(report,indent=2))
 if process is not None:
  os.killpg(process.pid,signal.SIGTERM)
  try:process.wait(timeout=10)
  except subprocess.TimeoutExpired:os.killpg(process.pid,signal.SIGKILL);process.wait()
