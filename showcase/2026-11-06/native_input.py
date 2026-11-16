"""Scroll or focus only inside the owned recording display and app."""
import ctypes
import json
import os
import sys
import time
from pathlib import Path
state = json.loads((Path(__file__).resolve().parent/'.private/ready.json').read_text())
assert state['display'] != os.environ.get('DISPLAY'), 'Recording display must be isolated'
command = json.load(sys.stdin)
x = ctypes.CDLL('libX11.so.6'); t = ctypes.CDLL('libXtst.so.6')
x.XOpenDisplay.argtypes=[ctypes.c_char_p]; x.XOpenDisplay.restype=ctypes.c_void_p
x.XFlush.argtypes=[ctypes.c_void_p]
d=x.XOpenDisplay(state['display'].encode())
t.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
t.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
if command['op']=='type':
    x.XStringToKeysym.argtypes=[ctypes.c_char_p];x.XStringToKeysym.restype=ctypes.c_ulong
    x.XKeysymToKeycode.argtypes=[ctypes.c_void_p,ctypes.c_ulong];x.XKeysymToKeycode.restype=ctypes.c_uint
    t.XTestFakeKeyEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
    for char in command['text']:
        name={' ':'space','-':'minus','.':'period','\n':'Return'}.get(char,char)
        code=x.XKeysymToKeycode(d,x.XStringToKeysym(name.encode()))
        if not code:raise RuntimeError('Unsupported key: '+repr(char))
        t.XTestFakeKeyEvent(d,code,1,0);t.XTestFakeKeyEvent(d,code,0,0)
        x.XFlush(d);time.sleep(.025)
elif command['op']=='scroll':
    t.XTestFakeMotionEvent(d,-1,command.get('x',1080),command.get('y',790),0)
    steps=command['steps']; button=5 if steps>0 else 4
    for i in range(abs(steps)):
        t.XTestFakeButtonEvent(d,button,1,0);t.XTestFakeButtonEvent(d,button,0,0)
        x.XFlush(d);time.sleep(.08)
    t.XTestFakeMotionEvent(d,-1,1279,1099,0);x.XFlush(d)
else:
    import gi
    gi.require_version('Atspi','2.0')
    from gi.repository import Atspi
    def walk(n):
        yield n
        try:
            for i in range(n.get_child_count()):yield from walk(n.get_child_at_index(i))
        except Exception:pass
    desktop=Atspi.get_desktop(0)
    apps=[desktop.get_child_at_index(i) for i in range(desktop.get_child_count())]
    app=next(a for a in apps if a.get_process_id()==state['pid'])
    n=next(n for n in walk(app) if n.get_name()==command['name'])
    if command['op']=='focus':Atspi.Component.grab_focus(n)
    elif command['op']=='scroll_to':Atspi.Component.scroll_to(n,Atspi.ScrollType.ANYWHERE)
    else:raise RuntimeError('Unknown operation')
time.sleep(.4)
