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
    if depth > 40: return
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
                if ' '.join(node.get_name().split()).startswith(' '.join(name.split())): return node
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
def type_into(name, text):
    node=find(name)
    if not Atspi.Component.grab_focus(node): raise RuntimeError('Cannot focus '+name)
    subprocess.run([str(Path(sys.argv[1]).parent/'examples/review_type_smoke'), text, 'replace'],check=True)
def wait_text(fragment):
    until=time.monotonic()+10
    while time.monotonic()<until:
        for node in walk(Atspi.get_desktop(0)):
            try:
                if fragment in node.get_name(): return
                if node.get_text_iface() and fragment in Atspi.Text.get_text(node,0,-1): return
            except Exception: pass
        time.sleep(.2)
    raise RuntimeError('Expected native text: '+fragment)
guide=output/'workspace/guide.md'
original=guide.read_text()
try:
    import sqlite3
    click('Native review fixture')
    click('Open settings')
    subprocess.run([str(Path(sys.argv[1]).parent/'examples/review_type_smoke'), '12', 'scroll'],check=True)
    time.sleep(.5)
    type_into('Document paths, one per line', 'guide.md')
    click('Enable project documents')
    wait_text('Project documents enabled')
    subprocess.run([str(Path(sys.argv[1]).parent/'examples/review_type_smoke'), '12', 'scroll'],check=True)
    time.sleep(.5)
    type_into('Test document retrieval','Alpha')
    click('Search documents')
    find('guide.md:1')
    guide.write_text('Gamma is a changed local document.\n')
    type_into('Test document retrieval','Gamma')
    click('Search documents')
    find('guide.md:1')
    guide.unlink()
    click('Search documents')
    wait_text('No matching current documents')
    click('Disable and delete selection')
    wait_text('Project document selection deleted')
    with sqlite3.connect(output/'data/app.locallm.desktop/locallm.sqlite') as db:
        saved=json.loads(db.execute("select value from settings where key='document_indexes_v1'").fetchone()[0])
    entry=saved[str((output/'workspace').resolve())]
    if entry['enabled'] or entry['paths']: raise RuntimeError('Selection was not deleted')
    result={'passed':True,'scope':'isolated project','optIn':True,'changedAndDeletedFiles':True,'disabledAndDeletedSelection':True,'networkUsed':False}
    (output/'native-document-error.txt').unlink(missing_ok=True)
    (output/'native-document-acceptance.json').write_text(json.dumps(result,indent=2))
    print('Native document acceptance:',json.dumps(result),flush=True)
except Exception as error:
    nodes=[]
    for node in walk(Atspi.get_desktop(0)):
        try: nodes.append({'name':node.get_name(),'role':node.get_role_name()})
        except Exception: pass
    (output/'native-documents-tree.json').write_text(json.dumps(nodes,indent=2))
    (output/'native-document-error.txt').write_text(str(error)); capture('native-documents-failure'); raise
finally:
    guide.write_text(original)
    app.terminate()
    try: app.wait(5)
    except subprocess.TimeoutExpired: app.kill(); app.wait()
