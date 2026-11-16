import json
import sys
import time
import uuid
from pathlib import Path

P = Path(__file__).resolve().parent / '.private'
commands = json.load(sys.stdin)
if isinstance(commands, dict): commands = [commands]
for command in commands:
    command['id'] = str(uuid.uuid4())
    temporary = P / 'command.tmp'
    temporary.write_text(json.dumps(command))
    temporary.replace(P / 'command.json')
    end = time.monotonic() + 25
    while time.monotonic() < end:
        try: response = json.loads((P/'response.json').read_text())
        except (FileNotFoundError,json.JSONDecodeError): time.sleep(.1); continue
        if response.get('id') == command['id']:
            if command['op'] == 'tree':
                for node in response.get('result') or []:
                    if not command.get('filter') or command['filter'].lower() in node['name'].lower(): print(json.dumps(node))
            else: print(json.dumps({'op':command['op'], 'name':command.get('name'), **response}))
            if not response.get('ok'): raise SystemExit(1)
            break
        time.sleep(.1)
    else: raise SystemExit('Native controller timed out')
