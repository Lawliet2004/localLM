#!/usr/bin/env python3
"""Seed only the owned isolated review database; never opens production app data."""
import hashlib, json, sqlite3, time
from pathlib import Path
root = Path('.local/qa/review-2026-10-17-native').resolve()
db = root / 'data/app.locallm.desktop/locallm.sqlite'
if not db.is_file(): raise SystemExit('Run isolated native smoke first.')
workspace = root / 'workspace'; workspace.mkdir(exist_ok=True)
content = 'Name,Latency\nAlpha,12\nBeta,18\n'
(workspace / 'latency.csv').write_text(content)
(workspace / 'guide.md').write_text('Alpha has 12 ms latency. Beta has 18 ms latency.\n')
now = int(time.time() * 1000)
c = sqlite3.connect(db)
id = 'native-review-fixture'
c.execute('delete from conversations where id=?', (id,))
c.execute('insert into conversations(id,title,updated_at) values(?,?,?)', (id, 'Native review fixture', now))
settings = {'workspace_index_v1': {'projects':[{'id':'native-review-project','name':'Review fixture','path':str(workspace)}], 'tasks':{id:{'projectId':'native-review-project','archived':False,'pinned':False}}}, 'workspace_path':str(workspace)}
for key,value in settings.items(): c.execute('insert or replace into settings(key,value) values(?,?)',(key,json.dumps(value)))
messages = [
 ('native-user','user','Inspect the supplied latency fixture and generated CSV.',''),
 ('native-web','tool',json.dumps({'request':{'connector':'Harness','name':'web_search','arguments':{'question':'Native fixture evidence'},'decision':'allowed'},'result':{'sources':[{'id':'A','url':'https://example.org/alpha','title':'Alpha fixture source','snippet':'Alpha: 12 ms.'},{'id':'B','url':'https://example.org/beta','title':'Beta fixture source','snippet':'Beta: 18 ms.'}]}}),''),
 ('native-files','tool',json.dumps({'request':{'connector':'Local execution','name':'run_code','arguments':{'language':'python','code':'# Native acceptance fixture. No generated code was executed.'},'decision':'allowed'},'result':{'workspace':str(workspace),'stdout':'Fixture files are available for preview.','exitCode':0,'artifacts':[{'path':'latency.csv','sha256':hashlib.sha256(content.encode()).hexdigest(),'bytes':len(content.encode())}]}}),''),
 ('native-answer','assistant','# Native review answer\n\nAlpha is **6 ms faster** than Beta. [A][B]\n\n| Sample | Latency |\n| --- | --- |\n| Alpha | 12 ms |\n| Beta | 18 ms |\n\n```python\ndef add(a, b):\n    return a + b\n```','Fixture reasoning without a recorded duration.'),
]
for i,(mid,role,text,reasoning) in enumerate(messages): c.execute('insert into messages(id,conversation_id,role,content,reasoning,status,created_at) values(?,?,?,?,?,?,?)',(mid,id,role,text,reasoning,'complete',now+i))
c.commit(); c.close()
print('Seeded isolated native review fixture:', db)
