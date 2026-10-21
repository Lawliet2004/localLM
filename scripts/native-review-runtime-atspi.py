#!/usr/bin/python3
"""Drive only the isolated review app through its real accessibility surface."""
import json, os, subprocess, sys, time
from pathlib import Path
import gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi
if os.environ.get('LOCALLM_DESKTOP_TEST_ISOLATED') != '1': raise SystemExit('Isolated display required.')
output = Path(sys.argv[2]); app = subprocess.Popen([sys.argv[1]])
results = []
def walk(node, depth=0):
    if depth > 18: return
    yield node
    try:
        for i in range(node.get_child_count()):
            child = node.get_child_at_index(i)
            if child is not None: yield from walk(child, depth+1)
    except Exception: pass
def find(name, within=None, timeout=15):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        for node in walk(within or Atspi.get_desktop(0)):
            try:
                if node.get_name() == name: return node
            except Exception: pass
        time.sleep(.15)
    raise RuntimeError('Accessible control not found: ' + name)
def click(name, within=None):
    node = find(name, within)
    interface = node.get_action_iface()
    if not interface or not interface.get_n_actions(): raise RuntimeError('No accessible action: ' + name)
    if not interface.do_action(0): raise RuntimeError('Action failed: ' + name)
    time.sleep(.5)
def capture(name):
    subprocess.run(['ffmpeg','-loglevel','error','-f','x11grab','-video_size','1440x1000','-i',os.environ['DISPLAY'],'-frames:v','1','-y',str(output/(name+'.png'))],check=True)
try:
    import sqlite3
    click('Models & runtime')
    click('Load model')
    find('Unload', timeout=100)
    click('Conversations')
    click('Native review fixture')
    combo = find('Reasoning for next response')
    if not combo.get_state_set().contains(Atspi.StateType.ENABLED): raise RuntimeError('Reasoning capability not enabled for runtime fallback template')
    entry = find('Message')
    if not Atspi.Component.grab_focus(entry): raise RuntimeError('Cannot focus native prompt')
    subprocess.run([str(Path(sys.argv[1]).parent/'examples/review_type_smoke'), 'What is 2 + 2? Answer with the number only.'], check=True)
    sent_at = int(time.time()*1000)
    click('Send message')
    database = output/'data/app.locallm.desktop/locallm.sqlite'
    row = None
    until = time.monotonic()+100
    while time.monotonic()<until:
        with sqlite3.connect(database) as db:
            row = db.execute("select id,content,reasoning,status from messages where role='assistant' and created_at>=? order by created_at desc limit 1",(sent_at,)).fetchone()
        if row and row[3] != 'streaming': break
        time.sleep(.3)
    if not row or row[3] != 'complete' or row[1].strip() != '4': raise RuntimeError('Native ordinary Quick response failed: '+repr(row))
    with sqlite3.connect(database) as db: metrics=json.loads(db.execute('select metrics_json from message_inference where message_id=?',(row[0],)).fetchone()[0])
    if not metrics['rounds'] or metrics['rounds'][0]['firstVisibleAnswerMs'] is None: raise RuntimeError('Visible-answer measurement missing')
    results.append({'name':'native ordinary Quick response','passed':True,'answer':row[1],'reasoning':row[2],'capabilityEnabled':True,'metrics':metrics})
    capture('native-runtime-quick-answer')
    click('Open settings')
    click('Settings: Diagnostics')
    click('Run local check')
    until = time.monotonic()+130
    report = None
    while time.monotonic()<until:
        try:
            detail=find('Recorded answers and raw report',timeout=.5)
            interface=detail.get_action_iface()
            if interface and interface.get_n_actions(): interface.do_action(0)
            for node in walk(Atspi.get_desktop(0)):
                try:
                    text=Atspi.Text.get_text(node,0,-1)
                    if text.lstrip().startswith('{') and '"schemaVersion"' in text:
                        candidate=json.loads(text)
                        if isinstance(candidate,dict) and candidate.get('schemaVersion')==1 and 'records' in candidate:report=candidate;break
                except Exception: pass
            if report:break
        except Exception: pass
        time.sleep(.5)
    if not report or len(report['records'])!=6: raise RuntimeError('In-app diagnostic did not return six real streams')
    measured=[record for record in report['records'] if record['cache']!='prime']
    if not all(record['complete'] and record['correct'] for record in measured): raise RuntimeError('Native diagnostic fixture failure: '+json.dumps(measured))
    (output/'native-in-app-diagnostic.json').write_text(json.dumps(report,indent=2))
    results.append({'name':'native in-app latency diagnostic','passed':True,'streams':len(report['records']),'measured':len(measured),'correct':sum(record['correct'] for record in measured)})
    capture('native-runtime-diagnostic-results')
    click('Unload')
    find('Load model')
    (output/'native-runtime-acceptance.json').write_text(json.dumps(results,indent=2))
    print('Native runtime acceptance:',json.dumps(results),flush=True)
except Exception as error:
    (output/'native-runtime-error.txt').write_text(str(error))
    capture('native-runtime-failure')
    raise

finally:
    app.terminate()
    try: app.wait(5)
    except subprocess.TimeoutExpired: app.kill(); app.wait()
