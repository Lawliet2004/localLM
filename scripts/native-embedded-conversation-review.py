#!/usr/bin/python3
"""Review the embedded native app on the authorized desktop with fixture data.

Run under a private D-Bus session and isolated XDG directories. Captures are
limited to the app's own X11 window; no desktop-wide screenshot is taken.
"""
import ctypes
import http.server
import json
import os
import re
import sqlite3
import subprocess
import sys
import threading
import time
from pathlib import Path

import gi
gi.require_version('Atspi', '2.0')
from gi.repository import Atspi
from PIL import Image

if os.environ.get('LOCALLM_EMBEDDED_REVIEW') != '1':
    raise SystemExit('Use the isolated embedded-review launcher.')
binary = Path(sys.argv[1]).resolve()
output = Path(sys.argv[2]).resolve()
data = Path(os.environ['XDG_DATA_HOME']).resolve()
if output not in data.parents or not binary.is_file():
    raise SystemExit('An embedded binary and isolated data directory are required.')
output.mkdir(parents=True, exist_ok=True)
workspace = output / 'workspace'
workspace.mkdir(exist_ok=True)
log = (output / 'native-app.log').open('a')
approval_fixture = os.environ.get('LOCALLM_REVIEW_APPROVAL') == '1'
server = None
if approval_fixture:
    class FixtureProvider(http.server.BaseHTTPRequestHandler):
        calls = 0
        def log_message(self, *_args): pass
        def do_POST(self):
            self.rfile.read(int(self.headers.get('Content-Length', '0')))
            if not self.path.endswith('/chat/completions'):
                body = b'{"input_tokens":10}'
                self.send_response(200); self.send_header('Content-Type', 'application/json'); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
                return
            FixtureProvider.calls += 1
            delta = {'choices': [{'delta': {'tool_calls': [{'index': 0, 'id': 'native-approval-call', 'type': 'function', 'function': {'name': 'workspace_create_file', 'arguments': json.dumps({'path': 'review-output.txt', 'content': 'fixture'})}}]}, 'finish_reason': 'tool_calls'}]} if FixtureProvider.calls == 1 else {'choices': [{'delta': {'content': 'The request was not run.'}, 'finish_reason': 'stop'}]}
            body = f'data: {json.dumps(delta)}\r\n\r\ndata: [DONE]\r\n\r\n'.encode()
            self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.send_header('Content-Length', str(len(body))); self.end_headers(); self.wfile.write(body)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), FixtureProvider)
    threading.Thread(target=server.serve_forever, daemon=True).start()

def start():
    return subprocess.Popen([str(binary)], stdout=log, stderr=log)

def stop(app):
    if app.poll() is None:
        app.terminate()
        try: app.wait(timeout=10)
        except subprocess.TimeoutExpired: app.kill(); app.wait()

def wait_for(predicate, description, seconds=45):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = predicate()
        if result: return result
        time.sleep(.25)
    raise RuntimeError(f'Timed out waiting for {description}.')

def database_ready(database):
    if not database.is_file(): return False
    try:
        with sqlite3.connect(f'file:{database}?mode=ro', uri=True, timeout=.2) as db:
            tables = {row[0] for row in db.execute("SELECT name FROM sqlite_master WHERE type='table'")}
            required = {'conversations', 'messages', 'settings', 'session_events'}
            if os.environ.get('LOCALLM_REVIEW_PENDING') == '1' or approval_fixture: required.add('pending_turns')
            return required.issubset(tables)
    except sqlite3.Error:
        return False

def window_for(pid):
    tree = subprocess.run(['xwininfo', '-root', '-tree'], capture_output=True, text=True, check=True).stdout
    for match in re.finditer(r'(0x[0-9a-f]+)\s+"LocalLM"', tree, re.I):
        identifier = match.group(1)
        prop = subprocess.run(['xprop', '-id', identifier, '_NET_WM_PID'], capture_output=True, text=True).stdout
        if re.search(r'=\s*' + str(pid) + r'\s*$', prop):
            detail = subprocess.run(['xwininfo', '-id', identifier], capture_output=True, text=True).stdout
            if 'Map State: IsViewable' in detail: return identifier
    return None

def walk(node, depth=0):
    if depth > 24: return
    yield node
    try:
        for index in range(node.get_child_count()):
            child = node.get_child_at_index(index)
            if child is not None: yield from walk(child, depth + 1)
    except Exception: pass

def app_accessible(pid):
    desktop = Atspi.get_desktop(0)
    for index in range(desktop.get_child_count()):
        app = desktop.get_child_at_index(index)
        try:
            if app and app.get_process_id() == pid: return app
        except Exception: pass
    return None

def find_node(app, label, last=False):
    def find():
        root = app_accessible(app.pid)
        if not root: return None
        found = []
        for node in walk(root):
            try:
                if node.get_name() == label:
                    if not last: return node
                    found.append(node)
            except Exception: pass
        return found[-1] if found else None
    return wait_for(find, f'accessible control {label}')

def click(app, label):
    node = find_node(app, label)
    action = node.get_action_iface()
    if not action or not action.get_n_actions() or not action.do_action(0): raise RuntimeError(f'Could not activate {label}.')
    time.sleep(1)

def capture(window, name):
    # Read the application window drawable itself, never the root desktop.
    x11 = ctypes.CDLL('libX11.so.6')
    display_p = ctypes.c_void_p
    drawable = ctypes.c_ulong
    x11.XOpenDisplay.argtypes = [ctypes.c_char_p]; x11.XOpenDisplay.restype = display_p
    x11.XCloseDisplay.argtypes = [display_p]
    x11.XGetGeometry.argtypes = [display_p, drawable, ctypes.POINTER(drawable), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_int), ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint), ctypes.POINTER(ctypes.c_uint)]
    x11.XGetImage.argtypes = [display_p, drawable, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
    x11.XGetImage.restype = ctypes.c_void_p
    x11.XDestroyImage.argtypes = [ctypes.c_void_p]
    x11.XSync.argtypes = [display_p, ctypes.c_int]
    handler_type = ctypes.CFUNCTYPE(ctypes.c_int, display_p, ctypes.c_void_p)
    errors = []
    handler = handler_type(lambda _display, _event: errors.append('X11 rejected window capture') or 0)
    x11.XSetErrorHandler.argtypes = [ctypes.c_void_p]; x11.XSetErrorHandler.restype = ctypes.c_void_p
    previous_handler = x11.XSetErrorHandler(ctypes.cast(handler, ctypes.c_void_p))
    class XImage(ctypes.Structure):
        _fields_ = [('width', ctypes.c_int), ('height', ctypes.c_int), ('xoffset', ctypes.c_int), ('format', ctypes.c_int), ('data', ctypes.c_void_p),
                    ('byte_order', ctypes.c_int), ('bitmap_unit', ctypes.c_int), ('bitmap_bit_order', ctypes.c_int), ('bitmap_pad', ctypes.c_int),
                    ('depth', ctypes.c_int), ('bytes_per_line', ctypes.c_int), ('bits_per_pixel', ctypes.c_int),
                    ('red_mask', ctypes.c_ulong), ('green_mask', ctypes.c_ulong), ('blue_mask', ctypes.c_ulong)]
    display = x11.XOpenDisplay(None)
    if not display: raise RuntimeError('Cannot open X11 display for window capture.')
    image = None
    try:
        root = drawable(); x = ctypes.c_int(); y = ctypes.c_int(); width = ctypes.c_uint(); height = ctypes.c_uint(); border = ctypes.c_uint(); depth = ctypes.c_uint()
        if not x11.XGetGeometry(display, int(window, 16), ctypes.byref(root), ctypes.byref(x), ctypes.byref(y), ctypes.byref(width), ctypes.byref(height), ctypes.byref(border), ctypes.byref(depth)):
            raise RuntimeError('Cannot inspect the application window.')
        image = x11.XGetImage(display, int(window, 16), 0, 0, width.value, height.value, ctypes.c_ulong(-1), 2)
        x11.XSync(display, 0)
        if not image or errors: raise RuntimeError(f'Window capture is unavailable on this desktop ({width.value}×{height.value}).')
        frame = ctypes.cast(image, ctypes.POINTER(XImage)).contents
        if (frame.bits_per_pixel, frame.byte_order, frame.red_mask, frame.green_mask, frame.blue_mask) != (32, 0, 0xff0000, 0xff00, 0xff):
            raise RuntimeError('Unsupported application window pixel format.')
        pixels = ctypes.string_at(frame.data, frame.bytes_per_line * frame.height)
        Image.frombytes('RGB', (frame.width, frame.height), pixels, 'raw', 'BGRX', frame.bytes_per_line).save(output / f'{name}.png')
    finally:
        if image: x11.XDestroyImage(image)
        x11.XCloseDisplay(display)
        x11.XSetErrorHandler(previous_handler)

app = start()
try:
    database = data / 'app.locallm.desktop' / 'locallm.sqlite'
    wait_for(lambda: database_ready(database) and window_for(app.pid), 'initialized native window')
    stop(app)
    now = int(time.time() * 1000)
    with sqlite3.connect(database) as db:
        ident = 'native-conversation-review'
        db.execute('INSERT INTO conversations(id,title,updated_at) VALUES(?,?,?)', (ident, 'Conversation review fixture', now))
        index = {'projects': [{'id': 'review-project', 'name': 'Review workspace', 'path': str(workspace)}],
                 'tasks': {ident: {'projectId': 'review-project', 'archived': False, 'pinned': False}}}
        for key, value in [('workspace_index_v1', index), ('workspace_path', str(workspace))]:
            db.execute('INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)', (key, json.dumps(value)))
        if approval_fixture:
            models = [{'id': 'fixture-model', 'contextLength': 32768, 'maxOutputTokens': 8192, 'toolSupport': 'supported', 'supportsImages': False}]
            db.execute('INSERT INTO providers(id,name,api_format,base_url,verified,models) VALUES(?,?,?,?,?,?)',
                       ('fixture-local', 'Scripted fixture', 'freetoken-openai', f'http://127.0.0.1:{server.server_port}/v1', 1, json.dumps(models)))
            db.execute('INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)', ('preferred_model', json.dumps({'providerId': 'fixture-local', 'modelId': 'fixture-model'})))
            db.execute('INSERT INTO conversation_tools(conversation_id,value) VALUES(?,?)', (ident, json.dumps({'accessMode': 'ask', 'sources': ['__workspace'], 'tools': []})))
        tools = [
            ('review-search', {'request': {'name': 'file_search', 'arguments': {'query': 'header'}, 'decision': 'allowed'}, 'result': {'matches': ['src/App.tsx']}}),
            ('review-read', {'request': {'name': 'read_file', 'arguments': {'path': 'src/App.tsx'}, 'decision': 'allowed'}, 'result': {'content': 'export function App() {}', 'path': 'src/App.tsx'}}),
            ('review-edit', {'request': {'name': 'edit_file', 'arguments': {'path': 'src/App.tsx'}, 'decision': 'allowed'}, 'result': {'diff': '--- a/src/App.tsx\n+++ b/src/App.tsx\n@@ -1 +1 @@\n-old\n+new\n'}}),
        ]
        rows = [('review-user', 'user', 'Improve the project header and explain what changed.', '')]
        rows.extend((ident, 'tool', json.dumps(record), '') for ident, record in tools)
        rows.append(('review-answer', 'assistant', 'I will inspect the header.Updated the header layout and kept the existing navigation.\n\nThe revised spacing works at narrow widths.\n\n```tsx\nexport function Header() { return <header>Project</header>; }\n```', 'Inspect the existing header.Decide how to preserve navigation.'))
        for offset, (message_id, role, content, reasoning) in enumerate(rows):
            db.execute('INSERT INTO messages(id,conversation_id,role,content,reasoning,status,created_at) VALUES(?,?,?,?,?,?,?)',
                       (message_id, 'native-conversation-review', role, content, reasoning, 'complete', now + offset))
        journal = [
            ('reasoning', {'text': 'Inspect the existing header.', 'durationMs': 1200}, None),
            ('model_response', {'content': 'I will inspect the header.', 'final': False}, None),
            ('tool_call', {'name': 'read_file', 'arguments': {'path': 'src/App.tsx'}}, 'read'),
            ('tool_result', {'name': 'read_file', 'result': {}}, 'read'),
            ('reasoning', {'text': 'Decide how to preserve navigation.', 'durationMs': 2300}, None),
            ('tool_call', {'name': 'edit_file', 'arguments': {'path': 'src/App.tsx'}}, 'edit'),
            ('tool_result', {'name': 'edit_file', 'result': {}}, 'edit'),
            ('model_response', {'content': 'Updated the header layout and kept the existing navigation.', 'final': True}, None),
        ]
        run_id = None
        if os.environ.get('LOCALLM_REVIEW_TYPED') == '1':
            run_id = 'native-typed-review-run'
            db.execute('INSERT INTO runs(id,conversation_id,status,created_at,updated_at) VALUES(?,?,?,?,?)', (run_id, ident, 'completed', now, now + 100))
            first = [
                ('item.started', {'item': {'id': 'thought-1', 'runId': run_id, 'round': 0, 'kind': 'reasoning', 'status': 'running', 'startedAt': now, 'payload': {}}}, None),
                ('item.delta', {'itemId': 'thought-1', 'delta': {'text': 'Inspect the existing header.'}}, None),
                ('item.completed', {'itemId': 'thought-1', 'status': 'completed', 'endedAt': now + 1200, 'durationMs': 1200}, None),
            ]
            second = [
                ('item.started', {'item': {'id': 'thought-2', 'runId': run_id, 'round': 1, 'kind': 'reasoning', 'status': 'running', 'startedAt': now + 1300, 'payload': {}}}, None),
                ('item.delta', {'itemId': 'thought-2', 'delta': {'text': 'Decide how to preserve navigation.'}}, None),
                ('item.completed', {'itemId': 'thought-2', 'status': 'completed', 'endedAt': now + 3600, 'durationMs': 2300}, None),
            ]
            # Omit legacy reasoning rows so the visible durations demonstrate
            # typed replay rather than the older fallback path.
            draft = []
            if os.environ.get('LOCALLM_REVIEW_DRAFT') == '1':
                draft = [
                    ('item.started', {'item': {'id': 'draft-1', 'runId': run_id, 'round': 0, 'kind': 'tool', 'status': 'running', 'startedAt': now + 10, 'payload': {'phase': 'preparing', 'name': 'workspace_edit_file', 'arguments': {'path': 'src/App.tsx'}}}}, None),
                    ('item.completed', {'itemId': 'draft-1', 'status': 'interrupted', 'endedAt': now + 20}, None),
                ]
            journal = first + draft + journal[1:4] + second + journal[5:]
        for seq, (kind, payload, call_id) in enumerate(journal, 1):
            db.execute('INSERT INTO session_events(id,conversation_id,run_id,seq,tool_call_id,event_type,payload,created_at) VALUES(?,?,?,?,?,?,?,?)',
                       (f'review-event-{seq}', ident, run_id, seq, call_id, kind, json.dumps(payload), now + seq))
        if os.environ.get('LOCALLM_REVIEW_PENDING') == '1' or approval_fixture:
            db.execute('INSERT INTO pending_turns(conversation_id,id,content,created_at) VALUES(?,?,?,?)',
                       (ident, 'native-pending-review', 'Create a fixture file for approval review.' if approval_fixture else 'Please check the remaining navigation states.', now + 20))
    app = start()
    window = wait_for(lambda: window_for(app.pid), 'fixture native window')
    wait_for(lambda: app_accessible(app.pid), 'application accessibility tree')
    click(app, 'localLM home')
    capture(window, 'home')
    click(app, 'Conversation review fixture')
    if approval_fixture:
        capture(window, 'approval-before-send')
        click(app, 'Send saved request')
        find_node(app, 'Allow Workspace to run this tool?')
        def approval_item_saved():
            with sqlite3.connect(f'file:{database}?mode=ro', uri=True) as db:
                return db.execute("SELECT 1 FROM session_events WHERE event_type='item.started' AND payload LIKE '%native-approval-call%' AND payload LIKE '%approval%' LIMIT 1").fetchone()
        wait_for(approval_item_saved, 'journal-backed approval item')
        if (workspace / 'review-output.txt').exists(): raise RuntimeError('Fixture file appeared before approval.')
        capture(window, 'approval-pending')
        click(app, 'Deny')
        if (workspace / 'review-output.txt').exists(): raise RuntimeError('Denied fixture file was created.')
        time.sleep(1)
        capture(window, 'approval-denied')
    capture(window, 'conversation-bottom')
    if os.environ.get('LOCALLM_REVIEW_PENDING') == '1':
        node = find_node(app, 'Saved unsent request')
        component = node.get_component_iface()
        if not component or not component.scroll_to(Atspi.ScrollType.TOP_EDGE):
            raise RuntimeError('Could not scroll to the saved-request panel.')
        time.sleep(1)
        capture(window, 'saved-request')
    node = find_node(app, 'user message')
    component = node.get_component_iface()
    if not component or not component.scroll_to(Atspi.ScrollType.TOP_EDGE):
        raise RuntimeError('Could not scroll to the start of the fixture conversation.')
    time.sleep(1)
    capture(window, 'conversation-top')
    node = find_node(app, 'assistant message', last=True)
    component = node.get_component_iface()
    if not component or not component.scroll_to(Atspi.ScrollType.TOP_EDGE):
        raise RuntimeError('Could not scroll to the fixture answer.')
    time.sleep(1)
    capture(window, 'conversation-answer')
    click(app, 'Conversation work')
    click(app, 'Worktree')
    capture(window, 'worktree')
    print('Captured native fixture windows:', output)
finally:
    stop(app)
    if server: server.shutdown(); server.server_close()
    log.close()
