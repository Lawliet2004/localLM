#!/usr/bin/python3
"""Actual embedded Tauri/WebKit + SQLite acceptance on an owned isolated display.
No browser, Vite URL, proxy, JavaScript injection or production database writes.
"""
import hashlib,json,os,sqlite3,subprocess,sys,time
from pathlib import Path
import gi
gi.require_version('Atspi','2.0')
from gi.repository import Atspi
if os.environ.get('LOCALLM_DESKTOP_TEST_ISOLATED')!='1':raise SystemExit('Isolated display required')
binary=Path(sys.argv[1]);output=Path(sys.argv[2]);output.mkdir(parents=True,exist_ok=True)
provenance=json.loads((output/'run-provenance.json').read_text())
with binary.open('rb') as built:
 digest=hashlib.file_digest(built,'sha256').hexdigest()
if digest!=provenance['binarySha256']:raise SystemExit('Launched native binary does not match recorded build fingerprint')
db=output/'data/app.locallm.desktop/locallm.sqlite'
app=None

def walk(node,depth=0):
 if depth>60:return
 yield node
 try:
  for i in range(node.get_child_count()):
   child=node.get_child_at_index(i)
   if child is not None:yield from walk(child,depth+1)
 except Exception:pass

def find(name,timeout=20,prefix=False):
 name=name.strip()
 end=time.monotonic()+timeout
 while time.monotonic()<end:
  for node in walk(Atspi.get_desktop(0)):
   try:
    if node.get_name().strip()==name or (prefix and node.get_role()==Atspi.Role.COMBO_BOX and node.get_name().strip().startswith(name+' ')):return node
   except Exception:pass
  time.sleep(.15)
 raise RuntimeError('Missing native accessible control: '+name)

def find_text(text,timeout=20):
 end=time.monotonic()+timeout
 while time.monotonic()<end:
  for node in walk(Atspi.get_desktop(0)):
   try:
    iface=node.get_text_iface()
    if iface and text in Atspi.Text.get_text(node,0,-1):return node
   except Exception:pass
  time.sleep(.15)
 raise RuntimeError('Missing native accessible text: '+text)

def click(name):
 node=find(name);action=node.get_action_iface()
 if not action or not action.get_n_actions() or not action.do_action(0):raise RuntimeError('Cannot activate '+name)
 time.sleep(.4)

def capture(name,width=1280,height=840):
 subprocess.run(['ffmpeg','-loglevel','error','-f','x11grab','-video_size',f'{width}x{height}','-i',os.environ['DISPLAY'],'-frames:v','1','-y',str(output/(name+'.png'))],check=True)

def process_peaks(root_pid):
 observed={}
 for entry in Path('/proc').iterdir():
  if not entry.name.isdigit():continue
  try:
   status=(entry/'status').read_text().splitlines()
   observed[int(entry.name)]={'parent':next(int(line.split()[1]) for line in status if line.startswith('PPid:')),'name':next(line.split(':',1)[1].strip() for line in status if line.startswith('Name:')),'peakRssKiB':next(int(line.split()[1]) for line in status if line.startswith('VmHWM:'))}
  except (OSError,StopIteration,ValueError):pass
 owned={root_pid}
 for _ in range(8):
  added={pid for pid,meta in observed.items() if meta['parent'] in owned}
  if added.issubset(owned):break
  owned.update(added)
 return {str(pid):observed[pid] for pid in owned if pid in observed}

def launch():
 global app
 app=subprocess.Popen([str(binary)])
 find('Conversations',timeout=30)

def stop():
 global app
 if app:
  app.terminate()
  try:app.wait(timeout=10)
  except subprocess.TimeoutExpired:app.kill();app.wait()
  app=None
try:
 if os.environ.get('LOCALLM_RESTORE_VISUAL_ONLY')=='1':
  with sqlite3.connect(db) as connection:
   title=connection.execute("SELECT title FROM conversations WHERE id='native-genui-model'").fetchone()[0]
   raw=connection.execute("SELECT p.record_json FROM message_presentations p JOIN messages m ON m.id=p.message_id WHERE m.conversation_id='native-genui-model' ORDER BY m.created_at DESC LIMIT 1").fetchone()[0]
   restored=json.loads(raw)
  if restored['generationStatus']!='complete':raise RuntimeError('Stored native model program is not complete')
  launch();find('Send message');capture('native-generative-ui-first-turn')
  click(title);find('Chart type',timeout=30,prefix=True)
  try:find('Jump to latest message',timeout=1);click('Jump to latest message')
  except RuntimeError:pass
  capture('native-generative-ui-actual-model-restored')
  (output/'restore-model-presentation.json').write_text(json.dumps(restored,indent=2))
  subprocess.run(['node','scripts/assert-genui-chart.mjs',str(output/'restore-model-presentation.json')],check=True)
  results={'historicalResultsFile':'acceptance.json','rerun':{'actualModelChartRestoredAfterRestart':True,'firstTurnComposer':True},'provenance':json.loads((output/'run-provenance.json').read_text())}
  (output/'restore-acceptance.json').write_text(json.dumps(results,indent=2));print(json.dumps(results),flush=True)
  sys.exit(0)
 launch();stop()
 workspace=output/'workspace';workspace.mkdir(exist_ok=True)
 with sqlite3.connect(db) as connection:
  now=int(time.time()*1000);cid='native-genui-acceptance';mid='native-genui-answer'
  connection.execute('PRAGMA foreign_keys=ON')
  connection.execute('DELETE FROM conversations WHERE id=?',(cid,))
  connection.execute('INSERT INTO conversations(id,title,updated_at) VALUES(?,?,?)',(cid,'Generative UI acceptance',now))
  connection.execute('INSERT INTO messages(id,conversation_id,role,content,reasoning,status,created_at) VALUES(?,?,?,?,?,?,?)',('native-genui-user',cid,'user','Native UI fixture: SQL joins quiz and binary search.','','complete',now))
  answer='LEFT JOIN preserves unmatched left rows. Binary search halves a sorted range.'
  connection.execute('INSERT INTO messages(id,conversation_id,role,content,reasoning,status,created_at) VALUES(?,?,?,?,?,?,?)',(mid,cid,'assistant',answer,'','complete',now+1))
  program='root = Response([Quiz("joins", "SQL joins", [{question:"Keeps unmatched left rows?",options:["Inner","Left"],answer:1,explanation:"LEFT JOIN keeps all left rows."}]), InteractiveExample("search", "binary-search", [2,4,6,8,10], 8)])'
  record={'messageId':mid,'format':'openui','formatVersion':1,'libraryVersion':'locallm.openui/2','sourceText':answer,'uiProgram':program,'interactiveState':{},'generationStatus':'complete','diagnostic':None}
  connection.execute('INSERT INTO message_presentations VALUES(?,?)',(mid,json.dumps(record)))
  for key,value in {'workspace_index_v1':{'projects':[{'id':'native-genui-project','name':'Native UI acceptance','path':str(workspace)}],'tasks':{cid:{'projectId':'native-genui-project','archived':False,'pinned':False}}},'workspace_path':str(workspace)}.items():connection.execute('INSERT OR REPLACE INTO settings VALUES(?,?)',(key,json.dumps(value)))
 launch();click('Generative UI acceptance');find('Send message');click('Left');find('Score: 1 / 1');capture('native-generative-ui-quiz');click('Next step');find('Step 2 / 3');capture('native-generative-ui')
 time.sleep(1)
 with sqlite3.connect(db) as connection:
  record=json.loads(connection.execute('SELECT record_json FROM message_presentations WHERE message_id=?',(mid,)).fetchone()[0])
  if record['interactiveState'].get('joins',{}).get('value',{}).get('0')!=1:raise RuntimeError('Native quiz did not persist exact selection')
  if record['interactiveState'].get('search-step',{}).get('value')!=1:raise RuntimeError('Native simulator did not persist independent step')
  if connection.execute('SELECT content FROM messages WHERE id=?',(mid,)).fetchone()[0]!=answer:raise RuntimeError('UI changed factual answer')
 latencies=[]
 for _ in range(10):
  for action,target in [('Previous','Step 1 / 3'),('Next step','Step 2 / 3')]:
   node=find(action);iface=node.get_action_iface();start=time.monotonic()
   if not iface.do_action(0):raise RuntimeError('Native action failed: '+action)
   find(target);latencies.append((time.monotonic()-start)*1000)
 memory={}
 for process_path in Path('/proc').iterdir():
  if not process_path.name.isdigit():continue
  try:
   status=(process_path/'status').read_text();parent=next(int(line.split()[1]) for line in status.splitlines() if line.startswith('PPid:'))
   if int(process_path.name)==app.pid or parent==app.pid:
    memory[process_path.name]={'name':next(line.split(':',1)[1].strip() for line in status.splitlines() if line.startswith('Name:')),'peakRssKiB':next(int(line.split()[1]) for line in status.splitlines() if line.startswith('VmHWM:'))}
  except (OSError,StopIteration,ValueError):pass
 interaction={'samples':len(latencies),'p50Ms':sorted(latencies)[len(latencies)//2],'p95Ms':sorted(latencies)[int((len(latencies)-1)*.95)],'method':'AT-SPI action to observed accessible step label; includes AT-SPI traversal/polling, not browser paint','processPeaks':memory}
 stop();launch();click('Generative UI acceptance');find('Score: 1 / 1');find('Step 2 / 3');capture('native-generative-ui-restored')
 click('Use dark theme');capture('native-generative-ui-dark')
 import ctypes,re
 tree=subprocess.check_output(['xwininfo','-root','-tree'],text=True)
 window=int(re.search(r'(0x[0-9a-f]+) "LocalLM"',tree).group(1),16)
 x11=ctypes.CDLL('libX11.so.6');x11.XOpenDisplay.restype=ctypes.c_void_p;x11.XOpenDisplay.argtypes=[ctypes.c_char_p];display=x11.XOpenDisplay(None)
 x11.XResizeWindow.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_uint,ctypes.c_uint];x11.XSync.argtypes=[ctypes.c_void_p,ctypes.c_int];x11.XCloseDisplay.argtypes=[ctypes.c_void_p]
 x11.XResizeWindow(display,window,760,840);x11.XSync(display,0);time.sleep(.6);capture('native-generative-ui-narrow-dark',760,840)
 x11.XResizeWindow(display,window,1280,840);x11.XSync(display,0);x11.XCloseDisplay(display);click('Use light theme')

 results={'interaction':interaction,'provenance':json.loads((output/'run-provenance.json').read_text()),'nativeRenderer':True,'quizScoring':True,'binarySearchStep':True,'sqliteState':True,'restartRestoration':True,'authoritativeAnswerUnchanged':True,'browserUsed':False,'fixture':'Manually authored deterministic component fixture; model benchmarks are separate.'}


 # Authored display-only fixtures prove native component behavior, never model reliability.
 stop()
 rich_answer='TCP: SYN then SYN-ACK then ACK. Files: src/a.ts, src/nested/b.ts. Jan sales: 10; Feb sales: 15.'
 rich_program='root = Response([Diagram("TCP",[{id:"a",label:"SYN"},{id:"b",label:"SYN-ACK"},{id:"c",label:"ACK"}],[{from:"a",to:"b"},{from:"b",to:"c"}]),FileTree(["src/a.ts","src/nested/b.ts"]),Slider("slope","Slope",-2,2,1),FunctionPlot("Linear","slope","linear",-2,2)])'
 with sqlite3.connect(db) as connection:
  connection.execute('PRAGMA foreign_keys=ON')
  for fixture_id,title,source,program,status in [('native-genui-rich','Native bounded experiences',rich_answer,rich_program,'complete'),('native-genui-invalid','Native expected fallback','The original fallback answer remains readable.','root = Response([Unknown("bad")])','complete'),('native-genui-interrupted','Native interrupted fallback','The interrupted original answer remains readable.','root = Response([Text("Partial")','streaming')]:
   connection.execute('INSERT INTO conversations(id,title,updated_at) VALUES(?,?,?)',(fixture_id,title,int(time.time()*1000)))
   message_id=fixture_id+'-answer'
   connection.execute('INSERT INTO messages(id,conversation_id,role,content,reasoning,status,created_at) VALUES(?,?,?,?,?,?,?)',(message_id,fixture_id,'assistant',source,'','complete',int(time.time()*1000)))
   fixture_record={'messageId':message_id,'format':'openui','formatVersion':1,'libraryVersion':'locallm.openui/2','sourceText':source,'uiProgram':program,'interactiveState':{},'generationStatus':status,'diagnostic':None}
   connection.execute('INSERT INTO message_presentations VALUES(?,?)',(message_id,json.dumps(fixture_record)))
   index=json.loads(connection.execute("SELECT value FROM settings WHERE key='workspace_index_v1'").fetchone()[0]);index['tasks'][fixture_id]={'projectId':'native-genui-project','archived':False,'pinned':False};connection.execute("UPDATE settings SET value=? WHERE key='workspace_index_v1'",(json.dumps(index),))
 launch();click('Native bounded experiences');find('Nodes and connections');find('src/');find('Slope');find('Chart type',prefix=True);capture('native-generative-ui-rich')
 results['nativeBoundedExperiences']=True
 click('Native expected fallback');find_text('The original fallback answer remains readable.');time.sleep(.5)
 with sqlite3.connect(db) as connection:
  rejected=json.loads(connection.execute('SELECT record_json FROM message_presentations WHERE message_id=?',('native-genui-invalid-answer',)).fetchone()[0])
  if rejected['generationStatus']!='error':raise RuntimeError('Malformed native UI rejection was not persisted')
 results['expectedMalformedFallback']=True
 click('Native interrupted fallback');find_text('The interrupted original answer remains readable.')
 with sqlite3.connect(db) as connection:
  interrupted=json.loads(connection.execute('SELECT record_json FROM message_presentations WHERE message_id=?',('native-genui-interrupted-answer',)).fetchone()[0])
  if interrupted['generationStatus']!='interrupted':raise RuntimeError('Restart did not settle interrupted presentation')
 results['expectedInterruptedFallback']=True
 capture('native-generative-ui-fallback')

 if os.environ.get('LOCALLM_MODEL_PREFS_DB'):
  stop()
  with sqlite3.connect(Path(os.environ['LOCALLM_MODEL_PREFS_DB']).resolve().as_uri()+'?mode=ro',uri=True) as original:
   settings={key:json.loads(value) for key,value in original.execute("SELECT key,value FROM settings WHERE key IN ('preferences','runtime')")}
  settings['runtime']['contextLength']=min(settings['runtime']['contextLength'],8192)
  settings['runtime']['gpuLayers']=0;settings['runtime']['visionEnabled']=False
  settings['preferences']['reasoningMode']='quick'
  settings['preferences']['projectorPath']='';settings['preferences']['maxTokens']=min(settings['preferences']['maxTokens'],1024)
  with sqlite3.connect(db) as connection:
   connection.execute('PRAGMA foreign_keys=ON')
   for key,value in settings.items():connection.execute('INSERT OR REPLACE INTO settings VALUES(?,?)',(key,json.dumps(value)))
   cid='native-genui-model';model_title='Native model visual answer'
   connection.execute('DELETE FROM conversations WHERE id=?',(cid,))
   connection.execute('INSERT INTO conversations(id,title,updated_at) VALUES(?,?,?)',(cid,model_title,int(time.time()*1000)))
   index=json.loads(connection.execute("SELECT value FROM settings WHERE key='workspace_index_v1'").fetchone()[0]);index['tasks'][cid]={'projectId':'native-genui-project','archived':False,'pinned':False}
   connection.execute("UPDATE settings SET value=? WHERE key='workspace_index_v1'",(json.dumps(index),))
   connection.execute('INSERT OR REPLACE INTO conversation_presentation_modes VALUES(?,?)',(cid,json.dumps('visual')))
   connection.execute('INSERT OR REPLACE INTO conversation_tools VALUES(?,?)',(cid,json.dumps({'sources':[],'tools':[],'accessMode':'ask'})))
  launch();click('Models & runtime');click('Load model');find('Unload',timeout=150);click('Close settings');click(model_title)
  entry=find('Message');Atspi.Component.grab_focus(entry)
  prompt='Make a bar chart of this numeric sales dataset: January sold 10 units; February sold 15 units; March sold 25 units. Each number is a unit count, not a date.'
  # Paste through native X11 input on this owned display. Per-character synthesis
  # can outrun WebKit's controlled textarea while inference is loading.
  subprocess.run(['xclip','-selection','clipboard'],input=prompt.encode(),check=True)
  keys=ctypes.CDLL('libXtst.so.6');x11.XOpenDisplay.restype=ctypes.c_void_p
  keyboard_display=x11.XOpenDisplay(None)
  x11.XStringToKeysym.argtypes=[ctypes.c_char_p];x11.XStringToKeysym.restype=ctypes.c_ulong
  x11.XKeysymToKeycode.argtypes=[ctypes.c_void_p,ctypes.c_ulong];x11.XKeysymToKeycode.restype=ctypes.c_ubyte
  keys.XTestFakeKeyEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
  def key(name,down):
   code=x11.XKeysymToKeycode(keyboard_display,x11.XStringToKeysym(name.encode()))
   keys.XTestFakeKeyEvent(keyboard_display,code,int(down),0);x11.XSync(keyboard_display,0);time.sleep(.05)
  for letter in ['a','v']:
   key('Control_L',True);key(letter,True);key(letter,False);key('Control_L',False)
  x11.XCloseDisplay(keyboard_display)
  time.sleep(.3);sent=int(time.time()*1000);click('Send message')
  with sqlite3.connect(db) as connection:
   submitted=connection.execute("SELECT content FROM messages WHERE conversation_id=? AND role='user' ORDER BY created_at DESC LIMIT 1",(cid,)).fetchone()
   if not submitted or submitted[0]!=prompt:raise RuntimeError('Native prompt input did not match the requested text')
  until=time.monotonic()+240;generated=None;first_accessible_visual=None;first_persisted_answer=None
  native_model_peaks={}
  while time.monotonic()<until:
   native_model_peaks.update(process_peaks(app.pid))
   with sqlite3.connect(db) as connection:
    row=connection.execute("SELECT p.record_json FROM message_presentations p JOIN messages m ON m.id=p.message_id WHERE m.conversation_id=? AND m.created_at>=? ORDER BY m.created_at DESC LIMIT 1",(cid,sent)).fetchone()
   if first_persisted_answer is None:
    with sqlite3.connect(db) as connection:
     answer_row=connection.execute("SELECT content FROM messages WHERE conversation_id=? AND role='assistant' AND created_at>=? ORDER BY created_at DESC LIMIT 1",(cid,sent)).fetchone()
    if answer_row and answer_row[0].strip():first_persisted_answer=int(time.time()*1000)-sent
   if first_accessible_visual is None:
    if any(node.get_name().strip().startswith('Chart type') for node in walk(Atspi.get_desktop(0))):first_accessible_visual=int(time.time()*1000)-sent
   if row:
    candidate=json.loads(row[0])
    if candidate['generationStatus']!='streaming':generated=candidate;break
   time.sleep(.3)
  if not generated:raise RuntimeError('Actual native model presentation did not finish')
  (output/'actual-model-presentation.json').write_text(json.dumps(generated,indent=2))
  results['actualNativeInference']={'timeToFirstAccessibleVisualMs':first_accessible_visual,'timeToFirstPersistedAuthoritativeAnswerMs':first_persisted_answer,'visualTimingMethod':'AT-SPI chart selector observation; not first paint','processPeaks':native_model_peaks,'config':{'contextLength':settings['runtime']['contextLength'],'cpuThreads':settings['runtime']['cpuThreads'],'gpuLayers':0,'maxTokens':settings['preferences']['maxTokens'],'modelSizeBytes':Path(settings['preferences']['modelPath']).stat().st_size},'status':generated['generationStatus'],'programChars':len(generated['uiProgram']),'sourceChars':len(generated['sourceText']),'model':Path(settings['preferences']['modelPath']).name,'cpuOnly':True,'reasoningMode':'quick'}
  results['scenarioFailures']=[]
  try:
   subprocess.run(['node','scripts/assert-genui-chart.mjs',str(output/'actual-model-presentation.json')],check=True)
   time.sleep(1)
   try:find('Jump to latest message',timeout=1);click('Jump to latest message')
   except RuntimeError:pass
   find('Chart type',timeout=10,prefix=True);results['actualModelChartRendered']=True
   capture('native-generative-ui-actual-model')
  except (RuntimeError,subprocess.CalledProcessError) as error:
   results['actualModelChartRendered']=False
   results['scenarioFailures'].append({'scenario':'actual-model-chart','failure':str(error)[:500]})
   capture('native-generative-ui-actual-model-failed')

  (output/'acceptance.json').write_text(json.dumps(results,indent=2))
  if os.environ.get('LOCALLM_NATIVE_TOOL_CASE')=='1':
   stop()
   actual_code='from collections import OrderedDict\n\nclass LRU:\n    def __init__(self, capacity):\n        self.capacity = capacity\n        self.cache = OrderedDict()\n\n    def get(self, key):\n        if key not in self.cache:\n            return None\n        self.cache.move_to_end(key)\n        return self.cache[key]\n'
   (workspace/'lru.py').write_text(actual_code)
   cid='native-genui-tool-model'
   with sqlite3.connect(db) as connection:
    connection.execute('PRAGMA foreign_keys=ON')
    connection.execute('INSERT INTO conversations(id,title,updated_at) VALUES(?,?,?)',(cid,'Native real file-read presentation',int(time.time()*1000)))
    index=json.loads(connection.execute("SELECT value FROM settings WHERE key='workspace_index_v1'").fetchone()[0]);index['tasks'][cid]={'projectId':'native-genui-project','archived':False,'pinned':False};connection.execute("UPDATE settings SET value=? WHERE key='workspace_index_v1'",(json.dumps(index),))
    connection.execute('INSERT OR REPLACE INTO conversation_presentation_modes VALUES(?,?)',(cid,json.dumps('visual')))
    connection.execute('INSERT OR REPLACE INTO conversation_tools VALUES(?,?)',(cid,json.dumps({'sources':['__workspace'],'tools':[],'accessMode':'autoApprove'})))
   launch();click('Models & runtime');click('Load model');find('Unload',timeout=150);click('Close settings');click('Native real file-read presentation')
   prompt='Use workspace_read_file to read lru.py now. Then show its exact full code verbatim, including the final newline, with the filename lru.py. Do not edit or execute it. Keep any explanation to one sentence.'
   Atspi.Component.grab_focus(find('Message'));subprocess.run(['xclip','-selection','clipboard'],input=prompt.encode(),check=True)
   keyboard_display=x11.XOpenDisplay(None)
   for letter in ['a','v']:
    key('Control_L',True);key(letter,True);key(letter,False);key('Control_L',False)
   x11.XCloseDisplay(keyboard_display);time.sleep(.3);tool_sent=time.monotonic();click('Send message')
   until=time.monotonic()+240;tool_generated=None
   while time.monotonic()<until:
    with sqlite3.connect(db) as connection:
     row=connection.execute("SELECT p.record_json FROM message_presentations p JOIN messages m ON m.id=p.message_id WHERE m.conversation_id=? ORDER BY m.created_at DESC LIMIT 1",(cid,)).fetchone()
    if row:
     candidate=json.loads(row[0])
     if candidate['generationStatus']!='streaming':tool_generated=candidate;break
    time.sleep(.3)
   if not tool_generated:raise RuntimeError('Actual tool-assisted presentation did not finish')
   (output/'actual-tool-presentation.json').write_text(json.dumps(tool_generated,indent=2))
   with sqlite3.connect(db) as connection:
    tool_rows=[content for content, in connection.execute("SELECT content FROM messages WHERE conversation_id=? AND role='tool'",(cid,))]
    if not any('workspace_read_file' in content and 'OrderedDict' in content for content in tool_rows):raise RuntimeError('Selected model did not actually read the file through the harness')
    metrics_row=connection.execute("SELECT metrics_json FROM message_inference WHERE message_id=?",(tool_generated['messageId'],)).fetchone()
   subprocess.run(['node','scripts/assert-genui-code.mjs',str(output/'actual-tool-presentation.json'),str(workspace/'lru.py')],check=True)
   find('Copy code block');capture('native-generative-ui-actual-tool-model')
   results['actualToolAssistedInference']={'workspaceReadVerified':True,'exactCodePreserved':True,'toolResultRows':len(tool_rows),'totalMs':(time.monotonic()-tool_sent)*1000,'metrics':json.loads(metrics_row[0]) if metrics_row else None}

 if results.get('scenarioFailures'):raise RuntimeError('Required native model scenarios failed: '+json.dumps(results['scenarioFailures']))
 results['overall']='passed'
 (output/'acceptance.json').write_text(json.dumps(results,indent=2));print(json.dumps(results),flush=True)
except Exception as error:
 if 'results' in locals():
  results['overall']='failed';results['failure']=str(error)[:500]
  (output/'acceptance.json').write_text(json.dumps(results,indent=2))
 (output/'acceptance-error.txt').write_text(str(error))
 try:capture('native-generative-ui-failure')
 except Exception:pass
 raise
finally:stop()
