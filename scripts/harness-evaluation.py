#!/usr/bin/env python3
"""Outcome evaluation through the production headless harness, with preflight token reservations.

Remote credentials stay in memory. All artifacts belong to synthetic trial workspaces.
Run --build baseline|candidate --pool remote|local. Repeat/resume skips finished trials.
"""
import argparse, hashlib, http.client, http.server, json, os, re, sqlite3, subprocess, threading, time, urllib.parse
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
QA=ROOT/'.local/qa/harness-simplification'
TASKS=json.loads((ROOT/'evaluations/harness/tasks.json').read_text())
REMOTE_LIMIT=20_000_000
TRIAL_LIMIT=250_000
FIXTURE_VERSION=2
class BudgetDenied(Exception):
 def __init__(self,scope):self.scope=scope;super().__init__(scope)

class Ledger:
 def __init__(self,path):
  self.path=path;self.lock=threading.Lock()
  with sqlite3.connect(path) as db:
   db.execute('CREATE TABLE IF NOT EXISTS requests(id TEXT PRIMARY KEY,pool TEXT,trial TEXT,reserved INTEGER,charged INTEGER,usage TEXT,status INTEGER,created REAL)')
 def reserve(self,pool,trial,amount):
  import uuid
  with self.lock,sqlite3.connect(self.path) as db:
   db.execute('BEGIN IMMEDIATE')
   used=db.execute('SELECT coalesce(sum(coalesce(charged,reserved)),0) FROM requests WHERE pool=?',(pool,)).fetchone()[0]
   trial_used=db.execute('SELECT coalesce(sum(coalesce(charged,reserved)),0) FROM requests WHERE pool=? AND trial=?',(pool,trial)).fetchone()[0]
   if pool=='remote' and used+amount>REMOTE_LIMIT:raise BudgetDenied('combined_remote')
   if trial_used+amount>TRIAL_LIMIT:raise BudgetDenied('trial')
   identity=str(uuid.uuid4());db.execute('INSERT INTO requests VALUES(?,?,?,?,NULL,NULL,NULL,?)',(identity,pool,trial,amount,time.time()));return identity
 def settle(self,identity,usage,status):
  # Missing usage consumes the entire reservation, even for failed requests.
  actual=None
  valid_count=lambda value:type(value) is int and value>=0
  if isinstance(usage,dict):
   if valid_count(usage.get('prompt_tokens')) and valid_count(usage.get('completion_tokens')):actual=usage['prompt_tokens']+usage['completion_tokens']
   elif valid_count(usage.get('total_tokens')):actual=usage['total_tokens']
  with self.lock,sqlite3.connect(self.path) as db:
   reserved=db.execute('SELECT reserved FROM requests WHERE id=?',(identity,)).fetchone()[0]
   db.execute('UPDATE requests SET charged=?,usage=?,status=? WHERE id=?',(actual if actual is not None else reserved,json.dumps(usage),status,identity))
 def report(self):
  with sqlite3.connect(self.path) as db:return [dict(pool=p,requests=n,chargedTokens=t) for p,n,t in db.execute('SELECT pool,count(*),sum(coalesce(charged,reserved)) FROM requests GROUP BY pool')]

def sha(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def record_changes(workspace):
 return {str(p.relative_to(workspace)):{'sha256':sha(p),'bytes':p.stat().st_size} for p in workspace.rglob('*') if p.is_file() and '__pycache__' not in p.parts}
def check(command,workspace,input=None):
 try:
  p=subprocess.run(command,cwd=workspace,input=input,capture_output=True,text=True,timeout=20)
  return {'passed':p.returncode==0,'exitCode':p.returncode,'stdout':p.stdout[-8000:],'stderr':p.stderr[-8000:]}
 except Exception as e:return {'passed':False,'error':str(e)}
def grade(task,workspace):
 kind=task['grade'];out={'factualAccuracy':'ungraded','visualInspection':'unverified'}
 try:
  if kind=='intervals':
   out.update(check(['python3','test_intervals.py'],workspace));out['testsUnchanged']=(workspace/'test_intervals.py').read_text()==task['seed']['test_intervals.py'];out['passed']=out['passed'] and out['testsUnchanged']
  elif kind=='lru':
   script="from cache import LRUCache\nc=LRUCache(2);c.put('a',1);c.put('b',2);c.put('a',3);assert c.get('b')==2;assert c.get('a')==3;c=LRUCache(2);c.put('a',1);c.put('b',2);assert c.get('a')==1;c.put('c',3);assert c.get('b') is None;c.put('a',4);assert c.get('c')==3;c.put('d',5);assert c.get('a') is None;assert c.get('d')==5\ntry:LRUCache(0)\nexcept ValueError:pass\nelse:raise AssertionError('invalid capacity accepted')"
   out.update(check(['python3','-c',script],workspace))
  elif kind=='csv':
   (workspace/'grade-input.csv').write_text('category,amount\n"a,b",1.20\nz,4\n"a,b",-0.10\n')
   good=check(['python3','summarize.py','grade-input.csv'],workspace)
   import csv,io
   rows=list(csv.reader(io.StringIO(good.get('stdout',''))));from decimal import Decimal
   good['passed']=good['passed'] and rows[0]==['category','total'] and [r[0] for r in rows[1:]]==['a,b','z'] and [Decimal(r[1]) for r in rows[1:]]==[Decimal('1.10'),Decimal('4')]
   (workspace/'grade-bad.csv').write_text('category,amount\na,invalid\n')
   bad=check(['python3','summarize.py','grade-bad.csv'],workspace)
   out.update(good);out['passed']=good['passed'] and bad.get('exitCode',0)!=0 and bool(bad.get('stderr'));out['malformedAmount']=bad
  elif kind=='svg':
   import xml.etree.ElementTree as ET
   source=(workspace/'badge.svg').read_text();element=ET.fromstring(source)
   out['passed']=element.tag.endswith('svg') and '512' in source and 'LOCAL' in source and bool(element.find('{http://www.w3.org/2000/svg}title') is not None) and bool(element.find('{http://www.w3.org/2000/svg}desc') is not None) and '<script' not in source and not re.search(r'(?:href|src)\s*=\s*[\'"](?:https?:|//)',source)
   out['structuralOnly']=True
  elif kind=='game':
   text=(workspace/'game.html').read_text();scripts=re.findall(r'<script[^>]*>(.*?)</script>',text,re.S|re.I)
   (workspace/'grade-game.js').write_text('\n'.join(scripts));out.update(check(['node','--check','grade-game.js'],workspace))
   out['passed']=out['passed'] and bool(scripts) and all(word.lower() in text.lower() for word in ['keydown','score','restart']) and not re.search(r'(?:src|href)=[\'"]https?://',text)
   out['structuralOnly']=True;out['interactiveBehavior']='ungraded'
  else:
   filename={'research':'research.md','brief':'brief.md','schedule':'schedule.md','data':'findings.md'}[kind]
   text=(workspace/filename).read_text();low=text.lower()
   if kind=='research':
    out['passed']=all(t.lower() in low for t in task['terms']) and task['domain'] in low
    out['sourceAccuracy']='requires review of actual source claims';out['structuralOnly']=True
   elif kind=='brief':
    # Wording is not an outcome: "no date has been set" is as valid as
    # "undecided". Review checks the actual decision, ownership and uncertainty.
    out['passed']=all(x in low for x in ['mira','jon','sam','security','attachment']) and len(text.split())<250
    out['structuralOnly']=True
   elif kind=='schedule':out['passed']=all(x in text for x in ['13:00','15:00','15:30','17:00']) and 'email' in low;out['structuralOnly']=True
   elif kind=='data':out['passed']='4' in text and '8' in text and '9' in text and 'missing' in low and 'b' in low;out['structuralOnly']=True
 except Exception as e:out.update(passed=False,error=str(e))
 return out

def main():
 parser=argparse.ArgumentParser();parser.add_argument('--build',choices=['baseline','candidate'],required=True);parser.add_argument('--revision',default='');parser.add_argument('--pool',choices=['remote','local'],required=True);parser.add_argument('--tasks',default='');parser.add_argument('--trials',type=int,default=3);parser.add_argument('--local-url',default='http://127.0.0.1:18491/v1');args=parser.parse_args()
 if args.tasks:
  unknown=set(args.tasks.split(','))-{task['id'] for task in TASKS}
  if unknown:parser.error('Unknown evaluation tasks: '+', '.join(sorted(unknown)))
 if args.trials<1:parser.error('trials must be positive')
 if args.revision and (args.build!='candidate' or not re.fullmatch(r'[a-z0-9-]+',args.revision)):raise ValueError('A follow-up revision must be a lowercase candidate name.')
 build_name=args.build+('-'+args.revision if args.revision else '')
 binary=QA/build_name/('eval-locallm' if args.build=='baseline' else 'locallm')
 if args.build=='candidate':
  manifest=json.loads((QA/build_name/'manifest.json').read_text())
  if sha(binary)!=manifest['binarySha256']:raise RuntimeError('Candidate binary differs from its frozen build manifest; freeze a completed build before evaluation.')
 output=QA/'evaluations';output.mkdir(exist_ok=True);ledger=Ledger(output/'tokens.sqlite')
 runner_source_sha256=sha(Path(__file__))
 credential={}
 if args.pool=='remote':
  # A private child-process pipe, deliberately never logged or saved.
  value=subprocess.run([str(binary),'--harness-eval','provider-config',str(Path.home()/'.local/share/app.locallm.desktop')],capture_output=True,text=True,check=True)
  credential=json.loads(value.stdout);base=credential['baseUrl'];model=credential['model']
 else:base=args.local_url;model='Gemma-4-E4B-Q2_K_XL'
 upstream=urllib.parse.urlsplit(base)
 class Proxy(http.server.BaseHTTPRequestHandler):
  def log_message(self,*args):pass
  def do_POST(self):
   trial=self.path.split('/')[1]
   if not re.fullmatch(r'[a-z0-9-]+',trial):self.send_error(400);return
   body=self.rfile.read(int(self.headers.get('Content-Length',0)))
   try:payload=json.loads(body)
   except ValueError:self.send_error(400);return
   reserve=len(body)+max(1,int(payload.get('max_tokens') or 4096))+4096
   try:identity=ledger.reserve(args.pool,trial,reserve)
   except BudgetDenied as denial:

    trial_dir=output/trial;trial_dir.mkdir(exist_ok=True)
    with (trial_dir/'budget-denials.jsonl').open('a') as log:log.write(json.dumps({'scope':denial.scope,'time':time.time(),'requestedReservation':reserve,'remoteLimit':REMOTE_LIMIT,'trialLimit':TRIAL_LIMIT})+'\n')
    self.send_response(429);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(b'{"error":{"code":"insufficient_quota","message":"Explicit evaluation token budget exhausted before request; no provider call sent."}}');return
   trial_dir=output/trial;trial_dir.mkdir(exist_ok=True)
   (trial_dir/(identity+'-request.json')).write_bytes(body)
   headers={'Content-Type':'application/json'}
   if credential:headers['Authorization']='Bearer '+credential['apiKey']
   connection=(http.client.HTTPSConnection if upstream.scheme=='https' else http.client.HTTPConnection)(upstream.hostname,upstream.port,timeout=300)
   usage=None;status=0;chunks=[]
   try:
    connection.request('POST',upstream.path.rstrip('/')+'/chat/completions',body,headers)
    response=connection.getresponse();status=response.status
    self.send_response(status);self.send_header('Content-Type',response.getheader('Content-Type','text/event-stream'));self.end_headers()
    while True:
     line=response.readline()
     if not line:break
     chunks.append(line)
     if line.startswith(b'data: '):
      try:
       item=json.loads(line[6:]);usage=item.get('usage') or usage
      except ValueError:pass
     try:self.wfile.write(line);self.wfile.flush()
     except (BrokenPipeError,ConnectionResetError):
      # A real cancelled harness closes its upstream connection. Do the same
      # for a single-slot local model instead of keeping cancelled generations
      # queued ahead of the next trial. Missing usage retains the reservation.
      if args.pool=='local':break
   except Exception as e:
    if not status:
     try:self.send_error(502,'Evaluation upstream transport failed')
     except OSError:pass
    (trial_dir/(identity+'-transport-error.txt')).write_text(type(e).__name__)
   finally:
    connection.close();ledger.settle(identity,usage,status)
    (trial_dir/(identity+'-response.sse')).write_bytes(b''.join(chunks))
 server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Proxy);threading.Thread(target=server.serve_forever,daemon=True).start()
 selected=[t for t in TASKS if not args.tasks or t['id'] in args.tasks.split(',')]
 try:
  for number in range(1,args.trials+1):
   for task in selected:
    identity=f"{build_name}-{args.pool}-{task['id']}-{number}-v{FIXTURE_VERSION}";trial=output/identity
    if (trial/'grade.json').exists():continue
    workspace=trial/'workspace';workspace.mkdir(parents=True,exist_ok=True)
    for name,content in task.get('seed',{}).items():(workspace/name).write_text(content)
    before=record_changes(workspace)
    request={'root':str(trial),'workspace':str(workspace),'buildIdentity':sha(binary),'prompt':task['prompt'],'baseUrl':f'http://127.0.0.1:{server.server_port}/{identity}/v1','model':model,'contextLength':32768,'maxOutputTokens':4096,'deadlineSeconds':600}
    (trial/'request.json').write_text(json.dumps(request,indent=2))
    (trial/'fixture.json').write_text(json.dumps({'version':FIXTURE_VERSION,'evaluationRunnerSha256':runner_source_sha256, 'taskDefinitionsSha256':sha(ROOT/'evaluations/harness/tasks.json'),'task':task['id'],'workload':task['workload'],'heldOut':task['heldOut'],'trialTokenAllowance':TRIAL_LIMIT,'combinedRemoteTokenLimit':REMOTE_LIMIT,'usageMissingPolicy':'full reservation','temperature':0.2,'runtimeTemplateSha256':sha(QA/'gemma-template-compatible.jinja') if args.pool=='local' else None},indent=2))
    started=time.monotonic()
    try:
     process=subprocess.run([str(binary),'--harness-eval',str(trial/'request.json')],capture_output=True,text=True,timeout=630)
     (trial/'runner.log').write_text(process.stdout+'\n'+process.stderr)
     after=record_changes(workspace);result=grade(task,workspace);result.update(task=task['id'],trial=number,build=args.build,pool=args.pool,runnerExit=process.returncode,elapsedSeconds=time.monotonic()-started,before=before,after=after,heldOut=task['heldOut'])
    except Exception as e:result={'passed':False,'error':str(e),'task':task['id'],'trial':number,'pool':args.pool,'build':args.build}
    if result.get('runnerExit') not in (None,0) and not (trial/'result.json').exists():raise RuntimeError(f'Fixture initialization failed in {trial}; stopping before more requests. Inspect runner.log.')
    denials=[json.loads(line) for line in (trial/'budget-denials.jsonl').read_text().splitlines()] if (trial/'budget-denials.jsonl').exists() else []
    result['evaluationIncomplete']=any(item.get('scope')=='combined_remote' for item in denials)
    if args.revision:result['revision']=args.revision
    (trial/'grade.json').write_text(json.dumps(result,indent=2));(output/'token-accounting.json').write_text(json.dumps(ledger.report(),indent=2))
    print(json.dumps({k:result.get(k) for k in ['task','trial','build','pool','passed','runnerExit','elapsedSeconds']}),flush=True)
    if result['evaluationIncomplete']:
     print('Combined remote budget cannot admit the next request. Matrix stopped; this trial is incomplete evidence, not a model-quality failure.',flush=True)
     break
   else:continue
   break
 finally:server.shutdown()
if __name__=='__main__':main()
