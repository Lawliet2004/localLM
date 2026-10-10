import json, io, sys, collections, os

def inspect(path, full=False):
    d = json.load(io.open(path, encoding='utf-8'))
    out = io.open(path.replace('.json', '.summary.txt'), 'w', encoding='utf-8', errors='replace')
    def p(*a): print(*a, file=out)
    p('test', d.get('testId'), d.get('slug'), '| model:', d.get('model'))
    p('elapsed:', round(d.get('elapsedMs',0)/60000,1), 'min | timedOut:', d.get('timedOut'), '| convo:', d.get('conversationId'))
    p('runs:', [(r.get('status'), r.get('error')) for r in d.get('runs',[])])
    p('runEvents:', len(d.get('runEvents',[])), '| sessionEvents:', len(d.get('sessionEvents',[])), '| chatEvents:', len(d.get('chatEvents',[])))
    et = collections.Counter(e.get('eventType') for e in d.get('sessionEvents',[]))
    p('sessionEventTypes:', dict(et))
    p('todos:', json.dumps(d.get('todos',[]))[:800])
    p('')
    p('=== MESSAGES ===')
    for i, m in enumerate(d.get('messages',[])):
        role, content = m.get('role'), m.get('content','') or ''
        p(f'--- [{i}] {role} status={m.get("status")} len={len(content)} ---')
        if role == 'tool':
            try:
                c = json.loads(content)
                req, res = c.get('request',{}), c.get('result',{})
                p(f'TOOL {req.get("name")} connector={req.get("connector")} decision={req.get("decision")} auth={req.get("authorization")}')
                args = json.dumps(req.get('arguments',{}))
                p('args:', args[:600])
                p('result:', json.dumps(res)[:1500])
            except Exception:
                p(content[:800])
        else:
            p(content[:4000 if full else 1200])
        p('')
    p('=== CHAT EVENT TYPES ===')
    ev = collections.Counter()
    for e in d.get('chatEvents',[]):
        keys = [k for k in ('notice','state','activity','approval','toolStream') if e.get(k)]
        ev[','.join(keys) or 'token'] += 1
    p(dict(ev))
    for e in d.get('chatEvents',[]):
        if e.get('notice') or e.get('approval'):
            p('EV:', json.dumps(e)[:400])
    out.close()
    return path.replace('.json', '.summary.txt')

if __name__ == '__main__':
    targets = sys.argv[1:] or sorted(
        os.path.join(r, f) for r, _, fs in os.walk('eval-results') for f in fs if f.endswith('.json'))
    for t in targets:
        if t.endswith('.json'):
            print(inspect(t))
