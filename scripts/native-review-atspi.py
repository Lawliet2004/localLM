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
    click('Native review fixture')
    click('2 tools used')
    click('Preview latency.csv')
    find('Copy latency.csv source')
    click('Open')
    find('Expanded latency.csv')
    capture('native-atspi-artifact-open')
    click('Close expanded preview')
    click('Save')
    time.sleep(.7)
    nodes=[]
    for node in walk(Atspi.get_desktop(0)):
        try: nodes.append({'name':node.get_name(),'role':node.get_role_name()})
        except Exception: pass
    (output/'atspi-save-tree.json').write_text(json.dumps(nodes,indent=2))
    chooser = find('Save artifact copy')
    entries=[]
    for node in walk(chooser):
        try:
            text = node.get_text_iface()
            if text and 'latency.csv' in Atspi.Text.get_text(node,0,-1): entries.append(node)
        except Exception: pass
    if not entries: raise RuntimeError('Native filename entry not found')
    destination = output / 'saved-latency.csv'
    entry = entries[0].get_editable_text_iface()
    if not entry or not Atspi.EditableText.set_text_contents(entries[0], str(destination)): raise RuntimeError('Cannot set native filename')
    click('Save', chooser)
    for _ in range(40):
        if destination.exists(): break
        time.sleep(.15)
    expected = (output/'workspace/latency.csv').read_bytes()
    actual = destination.read_bytes()
    if actual != expected: raise RuntimeError('Saved artifact bytes differ')
    results.append({'name':'native preview/open/save','passed':True,'bytes':len(actual),'sha256':__import__('hashlib').sha256(actual).hexdigest()})
    capture('native-atspi-artifact-saved')
    (output/'native-acceptance-error.txt').unlink(missing_ok=True)
    (output/'native-acceptance.json').write_text(json.dumps(results,indent=2))
    print('Native accessibility acceptance:',json.dumps(results),flush=True)
except Exception as error:
    (output/'native-acceptance-error.txt').write_text(str(error))
    capture('native-atspi-failure')
    raise

finally:
    app.terminate()
    try: app.wait(5)
    except subprocess.TimeoutExpired: app.kill(); app.wait()
