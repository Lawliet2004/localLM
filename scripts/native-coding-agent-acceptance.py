#!/usr/bin/python3
"""Native persisted transcript acceptance on the launcher's isolated display."""
import ctypes, json, os, re, sqlite3, subprocess, sys, time
from pathlib import Path
import gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi
if os.environ.get('LOCALLM_DESKTOP_TEST_ISOLATED') != '1':
    raise SystemExit('Isolated display required')
binary, output = Path(sys.argv[1]), Path(sys.argv[2])
db = output / 'data/app.locallm.desktop/locallm.sqlite'
app = None

def walk(node, depth=0):
    if depth > 60: return
    yield node
    try:
        for i in range(node.get_child_count()):
            child = node.get_child_at_index(i)
            if child is not None: yield from walk(child, depth+1)
    except Exception: pass

def find(name, timeout=25, prefix=False):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        for node in walk(Atspi.get_desktop(0)):
            try:
                if node.get_name().strip() == name or (prefix and node.get_name().strip().startswith(name)): return node
            except Exception: pass
        time.sleep(.15)
    raise RuntimeError('Missing accessible control: ' + name)

def click(name, prefix=False):
    node = find(name, prefix=prefix)
    action = node.get_action_iface()
    if not action or not action.get_n_actions() or not action.do_action(0):
        raise RuntimeError('Could not activate ' + name)
    time.sleep(.4)

def launch():
    global app
    app = subprocess.Popen([str(binary)])
    find('New conversation')

def stop():
    global app
    if app:
        app.terminate()
        try: app.wait(timeout=10)
        except subprocess.TimeoutExpired: app.kill(); app.wait()
        app = None

def capture(name, width, height):
    subprocess.run(['ffmpeg', '-loglevel', 'error', '-f', 'x11grab', '-draw_mouse', '0', '-video_size', f'{width}x{height}', '-i', os.environ['DISPLAY'], '-frames:v', '1', '-y', str(output / (name + '.png'))], check=True)

def resize(width, height):
    tree = subprocess.check_output(['xwininfo', '-root', '-tree'], text=True)
    window = int(re.search(r'(0x[0-9a-f]+) "LocalLM"', tree).group(1), 16)
    x11 = ctypes.CDLL('libX11.so.6')
    x11.XOpenDisplay.restype = ctypes.c_void_p
    display = x11.XOpenDisplay(None)
    x11.XMoveResizeWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint]
    x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
    x11.XCloseDisplay.argtypes = [ctypes.c_void_p]
    x11.XMoveResizeWindow(display, window, 0, 0, width, height)
    x11.XSync(display, 0); x11.XCloseDisplay(display)
    time.sleep(.6)

try:
    launch(); stop()
    workspace = output / 'workspace'
    workspace.mkdir(exist_ok=True)
    with sqlite3.connect(db) as connection:
        connection.execute('PRAGMA foreign_keys=ON')
        now = int(time.time()*1000)
        cid, run = 'coding-workflow', 'coding-run'
        connection.execute('DELETE FROM conversations WHERE id=?', (cid,))
        connection.execute('INSERT INTO conversations(id,title,updated_at) VALUES(?,?,?)', (cid, 'Coding workflow fixture', now))
        connection.execute('INSERT INTO runs(id,conversation_id,status,created_at,updated_at) VALUES(?,?,?,?,?)', (run,cid,'completed',now,now+6000))
        def message(mid, role, content, reasoning='', at=0):
            connection.execute('INSERT INTO messages(id,conversation_id,role,content,reasoning,status,created_at) VALUES(?,?,?,?,?,?,?)', (mid,cid,role,content,reasoning,'complete',now+at))
        message('coding-user','user','Read value.txt, fix it, run the check, and repair any failures.')
        seq = 0
        def event(kind, payload, step=None, call=None):
            global seq
            seq += 1
            connection.execute('INSERT INTO session_events VALUES(?,?,?,?,?,?,?,?,?,?)', (f'coding-event-{seq}',cid,run,seq,step,call,kind,json.dumps(payload),0,now+seq*100))
        event('turn_start', {})
        event('reasoning', {'text':'Fixture reasoning: inspect the file before editing.'}, 'one')
        event('model_response', {'content':'I will inspect the file and run its check.', 'final':False}, 'one')
        operations = [
            ('one','workspace_read_file',{'path':'value.txt'}, {'content':'broken\n'}, 'success'),
            ('two','workspace_edit_file',{'path':'value.txt'}, {'path':'value.txt','bytesWritten':13,'diff':'--- a/value.txt\n+++ b/value.txt\n@@ -1 +1 @@\n-broken\n+still-broken'}, 'success'),
            ('three','check_run',{'command':'sh check.sh'}, {'logs':{'stdout':'1..1\nnot ok 1 - value\n'},'exitCode':1}, 'failure'),
            ('four','workspace_edit_file',{'path':'value.txt'}, {'path':'value.txt','bytesWritten':6,'diff':'--- a/value.txt\n+++ b/value.txt\n@@ -1 +1 @@\n-still-broken\n+fixed'}, 'success'),
            ('five','check_run',{'command':'sh check.sh'}, {'logs':{'stdout':'1..1\nok 1 - value\n'},'exitCode':0}, 'success'),
        ]
        for step,name,args,data,status in operations:
            request = {'name':name,'arguments':args,'callId':'reused','runId':run,'stepId':step,'decision':'allowed'}
            result = {'schemaVersion':'locallm.tool-envelope/1','status':status,'data':data,'artifacts':[],'metadata':{},'error':{'code':'check-failed','message':'The first check failed.'} if status=='failure' else None}
            event('tool_call', request, step, 'reused')
            event('tool_result', {'name':name,'result':result}, step,'reused')
            message('saved-'+step,'tool',json.dumps({'request':request,'result':result}),at=seq*100)
        event('model_response', {'content':'Fixed value.txt. The first check failed; after repair, one test passed.', 'final':True}, 'six')
        event('verification_status', {'status':'passed','message':'The repaired file passed 1 test.'})
        event('turn_end', {'status':'completed'})
        message('coding-answer','assistant','I will inspect the file and run its check.Fixed value.txt. The first check failed; after repair, one test passed.','Fixture reasoning: inspect the file before editing.',at=seq*100)
        settings = {'workspace_index_v1':{'projects':[{'id':'coding-project','name':'Coding fixture','path':str(workspace)}],'tasks':{cid:{'projectId':'coding-project','archived':False,'pinned':False}}},'workspace_path':str(workspace)}
        for key,value in settings.items(): connection.execute('INSERT OR REPLACE INTO settings VALUES(?,?)', (key,json.dumps(value)))
    launch(); click('Coding workflow fixture'); find('Message')
    try: find('Use light theme', timeout=1); click('Use light theme')
    except RuntimeError: pass
    for theme in ['light','dark']:
        if theme=='dark': click('Use dark theme')
        for name,width,height in [('desktop',1440,1000),('narrow',760,840)]:
            resize(width,height)
            capture(f'{name}-{theme}-transcript',width,height)
    # Exercise the actual disclosure after the four representative captures.
    resize(1440,1000)
    click('1 file changed', prefix=True)
    click('View changes')
    capture('desktop-dark-expanded-changes',1440,1000)
    stop(); launch(); click('Coding workflow fixture'); find('Message')
    capture('desktop-dark-restored',1280,840)
    (output/'acceptance.json').write_text(json.dumps({'nativeRenderer':True,'themes':['light','dark'],'sizes':[[1440,1000],[760,840]],'fileChangesDisclosure':True,'restartRestoration':True,'browserUsed':False,'fixture':'authored persisted transcript; actual execution covered by Rust acceptance'},indent=2))
except Exception as error:
    (output/'error.txt').write_text(str(error))
    raise
finally:
    stop()
