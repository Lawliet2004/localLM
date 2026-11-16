"""Record the real LocalLM desktop app on an isolated X11 display.

The source profile is opened read-only for provider configuration only. Keys
remain encrypted. The recording profile and its credential copy are private
temporary files and are removed after capture.
"""
import ctypes
import json
import os
import re
import shutil
import sqlite3
import subprocess
import time
from pathlib import Path

import gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi

OUT = Path(__file__).resolve().parent
REPO = OUT.parents[1]
PRIVATE = OUT / '.private'
PRIVATE.mkdir(mode=0o700, exist_ok=True)
CAP = OUT / 'captures'
CAP.mkdir(exist_ok=True)
WS = OUT / 'demo-workspace'
PROVIDER = 'c7a146d5-4298-4f0f-b4b0-529b0f072b98'
CID = 'showcase-launch-report'
dbpath = PRIVATE / 'data/app.locallm.desktop/locallm.sqlite'
log = (PRIVATE / 'native-app.log').open('a')
xvfb = subprocess.Popen([str(REPO / '.local/native-tools/extracted/usr/bin/Xvfb'),
    '-displayfd', '1', '-screen', '0', '1280x1100x24', '-nolisten', 'tcp', '-ac', '-noreset'],
    stdout=subprocess.PIPE, stderr=log, text=True)
display = ':' + xvfb.stdout.readline().strip()
env = {**os.environ, 'DISPLAY': display, 'WAYLAND_DISPLAY': '', 'GDK_BACKEND': 'x11',
    'XDG_SESSION_TYPE': 'x11', 'XDG_DATA_HOME': str(PRIVATE / 'data'),
    'XDG_CONFIG_HOME': str(PRIVATE / 'config'), 'XDG_CACHE_HOME': str(PRIVATE / 'cache'),
    'LOCALLM_DESKTOP_TEST_ISOLATED': '1', 'NO_AT_BRIDGE': '0',
    'GTK_MODULES': 'gail:atk-bridge', 'LIBGL_ALWAYS_SOFTWARE': '1',
    'WEBKIT_DISABLE_COMPOSITING_MODE': '1'}
os.environ.update({k: env[k] for k in ['DISPLAY', 'WAYLAND_DISPLAY', 'GDK_BACKEND', 'NO_AT_BRIDGE']})
binary = REPO / 'src-tauri/target/debug/locallm'
app = None
recorder = None
record_started = None
events = []

def launch():
    global app
    app = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)

def stop_app():
    if app and app.poll() is None:
        app.terminate()
        try: app.wait(timeout=8)
        except subprocess.TimeoutExpired: app.kill(); app.wait()

def wait_for(fn, seconds=30):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        value = fn()
        if value: return value
        if app and app.poll() is not None: raise RuntimeError('Native app exited; inspect private log')
        time.sleep(.2)
    raise RuntimeError('Timed out waiting for native application')

def window_id():
    tree = subprocess.run(['xwininfo', '-root', '-tree'], env=env, capture_output=True, text=True).stdout
    for m in re.finditer(r'(0x[0-9a-f]+)\s+"LocalLM"', tree, re.I):
        prop = subprocess.run(['xprop', '-id', m[1], '_NET_WM_PID'], env=env, capture_output=True, text=True).stdout
        if re.search(r'=\s*' + str(app.pid) + r'\s*$', prop): return m[1]

def walk(node, depth=0):
    if depth > 28: return
    yield node
    try:
        for i in range(node.get_child_count()):
            child = node.get_child_at_index(i)
            if child: yield from walk(child, depth+1)
    except Exception: pass

def root():
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        a = desktop.get_child_at_index(i)
        try:
            if a and a.get_process_id() == app.pid: return a
        except Exception: pass

def nodes():
    r = root()
    return list(walk(r)) if r else []

def find(name, role=None, seconds=15):
    def match():
        for n in nodes():
            try:
                if n.get_name() == name and (not role or n.get_role_name() == role): return n
            except Exception: pass
    return wait_for(match, seconds)

def click(name, role=None):
    n = find(name, role)
    a = n.get_action_iface()
    if not a or not a.get_n_actions() or not a.do_action(0):
        raise RuntimeError('Cannot activate native control: ' + name)
    time.sleep(.45)

x11 = ctypes.CDLL('libX11.so.6')
x11.XOpenDisplay.argtypes = [ctypes.c_char_p]; x11.XOpenDisplay.restype = ctypes.c_void_p
x11.XStringToKeysym.argtypes = [ctypes.c_char_p]; x11.XStringToKeysym.restype = ctypes.c_ulong
x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]; x11.XKeysymToKeycode.restype = ctypes.c_uint
x11.XFlush.argtypes = [ctypes.c_void_p]
x11.XMoveResizeWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint]
xtest = ctypes.CDLL('libXtst.so.6')
xtest.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
xd = x11.XOpenDisplay(display.encode())

def key(keys):
    codes = [x11.XKeysymToKeycode(xd, x11.XStringToKeysym(k.encode())) for k in keys]
    for c in codes: xtest.XTestFakeKeyEvent(xd, c, 1, 0)
    for c in reversed(codes): xtest.XTestFakeKeyEvent(xd, c, 0, 0)
    x11.XFlush(xd)
    time.sleep(.2)

def text(name, value):
    n = find(name)
    if not Atspi.Component.grab_focus(n): raise RuntimeError('Cannot focus ' + name)
    # X11 clipboard paste produces real input events for React controlled fields.
    subprocess.run(['xclip', '-selection', 'clipboard'], input=value.encode(), env=env, check=True)
    key(['Control_L', 'a'])
    key(['Control_L', 'v'])
    time.sleep(.4)

def snap(name):
    subprocess.run(['ffmpeg', '-loglevel', 'error', '-f', 'x11grab', '-video_size', '1280x1100',
        '-i', display, '-frames:v', '1', '-update', '1', '-y', str(CAP / (name + '.png'))], check=True)

def record_stop():
    global recorder
    if recorder:
        recorder.communicate(input=b'q', timeout=15)
        recorder = None

def tree():
    result = []
    for n in nodes():
        try:
            name = n.get_name()
            if not name: continue
            e = n.get_component_iface()
            rect = Atspi.Component.get_extents(n, Atspi.CoordType.SCREEN) if e else None
            result.append({'name': name[:350], 'role': n.get_role_name(),
                'enabled': n.get_state_set().contains(Atspi.StateType.ENABLED),
                'rect': [rect.x, rect.y, rect.width, rect.height] if rect else None})
        except Exception: pass
    return result

try:
    launch()
    wait_for(lambda: dbpath.exists() and window_id())
    time.sleep(2)
    stop_app()
    source = sqlite3.connect('file:/home/papan/.local/share/app.locallm.desktop/locallm.sqlite?mode=ro', uri=True)
    provider = source.execute('SELECT id,name,api_format,base_url,verified,last_tested_at,models FROM providers WHERE id=?', (PROVIDER,)).fetchone()
    selected = source.execute("SELECT value FROM settings WHERE key='preferred_model'").fetchone()[0]
    source.close()
    now = int(time.time()*1000)
    with sqlite3.connect(dbpath) as db:
        db.execute('INSERT OR REPLACE INTO providers(id,name,api_format,base_url,verified,last_tested_at,models) VALUES(?,?,?,?,?,?,?)', provider)
        index = {'projects': [{'id': 'launch-lab', 'name': 'Launch Lab', 'path': str(WS)}],
            'tasks': {CID: {'projectId': 'launch-lab', 'archived': False, 'pinned': False}}}
        prefs = {'runtimePath': '', 'modelPath': '', 'projectorPath': '', 'temperature': .7,
            'topP': .95, 'maxTokens': 2048, 'reasoningMode': 'quick', 'generativeUi': False,
            'systemPrompt': 'You are a helpful assistant. Use the enabled tools to work on the supplied workspace. Keep the final answer concise and factual.'}
        for k,v in [('workspace_index_v1', json.dumps(index)), ('workspace_path', json.dumps(str(WS))),
            ('preferred_model', selected), ('preferences', json.dumps(prefs))]:
            db.execute('INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)', (k,v))
        choice = json.loads(selected)
        db.execute('INSERT OR REPLACE INTO conversations(id,title,updated_at,provider_id,model_id) VALUES(?,?,?,?,?)',
            (CID, 'Launch campaign report', now, choice['providerId'], choice['modelId']))
        db.execute('INSERT OR REPLACE INTO conversation_tools(conversation_id,value) VALUES(?,?)',
            (CID, json.dumps({'accessMode': 'autoApprove', 'sources': ['__workspace'], 'tools': []})))
    launch()
    wid = wait_for(window_id)
    x11.XMoveResizeWindow(xd, int(wid,16), 0, 0, 1280, 1100); x11.XFlush(xd)
    wait_for(root)
    time.sleep(3)
    (PRIVATE / 'ready.json').write_text(json.dumps({'pid': app.pid, 'display': display, 'window': wid}))
    print('Native recording session ready.', flush=True)
    (PRIVATE/'response.json').write_text(json.dumps({'ready': True}))
    last_id = None
    while True:
        command_file = PRIVATE / 'command.json'
        if not command_file.exists(): time.sleep(.15); continue
        try: cmd = json.loads(command_file.read_text())
        except json.JSONDecodeError: time.sleep(.1); continue
        if cmd.get('id') == last_id: time.sleep(.1); continue
        last_id = cmd.get('id')
        try:
            op = cmd['op']; result = None
            if op == 'tree': result = tree()
            elif op == 'click': click(cmd['name'], cmd.get('role'))
            elif op == 'text': text(cmd['name'], cmd['value'])
            elif op == 'key': key(cmd['keys'])
            elif op == 'snap': snap(cmd['name'])
            elif op == 'wait': time.sleep(min(float(cmd.get('seconds',1)),10))
            elif op == 'record':
                record_stop()
                recorder = subprocess.Popen(['ffmpeg', '-y', '-loglevel', 'error', '-f', 'x11grab',
                    '-framerate', '30', '-video_size', '1280x1100', '-draw_mouse', '0', '-i', display,
                    '-an', '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '16', '-pix_fmt', 'yuv420p',
                    str(CAP/(cmd['name']+'.mp4'))], stdin=subprocess.PIPE, stdout=log, stderr=log)
                record_started = time.monotonic()
                time.sleep(.6)
            elif op == 'record_stop': record_stop()
            elif op == 'stop': record_stop(); break
            else: raise RuntimeError('Unknown recording operation')
            events.append({'op':op,'name':cmd.get('name'), 'time':time.time(),
                'recordOffset':time.monotonic()-record_started if record_started else None})
            (PRIVATE/'events.json').write_text(json.dumps(events,indent=2))
            response = {'id': last_id, 'ok': True, 'result': result}
        except Exception as error:
            response = {'id': last_id, 'ok': False, 'error': str(error)}
        (PRIVATE/'response.json').write_text(json.dumps(response))
finally:
    record_stop()
    stop_app()
    xvfb.terminate()
    try: xvfb.wait(timeout=5)
    except subprocess.TimeoutExpired: xvfb.kill()
    shutil.rmtree(PRIVATE/'data/app.locallm.desktop/credentials', ignore_errors=True)
    log.close()
