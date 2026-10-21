"""Local stdio LSP fixture using only Python's standard library."""
import json, os, pathlib, re, subprocess, sys, time, urllib.parse
mode, log_path = sys.argv[1:3]
documents = {}
root = None

def log(method, **values):
    with open(log_path, 'a', encoding='utf-8') as out:
        out.write(json.dumps({'pid': os.getpid(), 'method': method, **values}) + '\n')

def send(value):
    body = json.dumps(value, ensure_ascii=False).encode('utf-8')
    sys.stdout.buffer.write(('Content-Length: %d\r\n\r\n' % len(body)).encode('ascii') + body)
    sys.stdout.buffer.flush()

def read():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line == b'\r\n':
            break
        key, value = line.decode('ascii').split(':', 1)
        headers[key.lower()] = value.strip()
    return json.loads(sys.stdin.buffer.read(int(headers['content-length'])))

def path(uri):
    parsed = urllib.parse.urlparse(uri)
    name = urllib.parse.unquote(parsed.path)
    if os.name == 'nt' and name.startswith('/') and len(name) > 2 and name[2] == ':':
        name = name[1:]
    return pathlib.Path(name)

def point(text, offset):
    before = text[:offset]
    return {'line': before.count('\n'), 'character': len(before.rsplit('\n', 1)[-1].encode('utf-16-le')) // 2}

def edits(text, new):
    return [{'range': {'start': point(text, m.start()), 'end': point(text, m.end())}, 'newText': new}
            for m in re.finditer(r'\bold_name\b', text)]

log('started')
if mode == 'descendant':
    child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
    log('descendant', child=child.pid)
if mode == 'stderr':
    sys.stderr.write('fixture diagnostics ' * 20000)
    sys.stderr.flush()
while True:
    message = read()
    if message is None:
        break
    method = message.get('method')
    params = message.get('params') or {}
    log(method or 'client_response', id=message.get('id'))
    if method == 'exit':
        break
    if method == 'initialize':
        root = path(params['rootUri'])
        if mode == 'bad_header':
            sys.stdout.buffer.write(b'Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}')
            sys.stdout.buffer.flush()
            time.sleep(60)
            continue
        if mode == 'configuration':
            send({'jsonrpc': '2.0', 'id': 'configuration-1', 'method': 'workspace/configuration', 'params': {'items': [{'section': 'fixture.answer'}]}})
            answer = read()
            log('configuration_result', result=answer.get('result'))
            send({'jsonrpc': '2.0', 'id': 'apply-1', 'method': 'workspace/applyEdit', 'params': {'edit': {'changes': {}}}})
            answer = read()
            log('apply_result', result=answer.get('result'))
        capabilities = {'textDocumentSync': {'openClose': True, 'change': 1}, 'documentSymbolProvider': True,
                        'executeCommandProvider': {'commands': ['fixture.refactor']}, 'referencesProvider': True, 'definitionProvider': True, 'hoverProvider': True,
                        'implementationProvider': True, 'codeActionProvider': {'resolveProvider': True}, 'renameProvider': {'prepareProvider': True}}
        if mode == 'impact': capabilities['callHierarchyProvider'] = True
        if mode not in ('push', 'unversioned', 'stale'):
            capabilities['diagnosticProvider'] = {'interFileDependencies': False, 'workspaceDiagnostics': False}
        if mode == 'utf8':
            capabilities['positionEncoding'] = 'utf-8'
        if mode == 'unsupported':
            capabilities = {}
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': {'capabilities': capabilities}})
    elif method == 'workspace/executeCommand':
        uri = next(iter(documents))
        send({'jsonrpc':'2.0','id':'command-edit','method':'workspace/applyEdit','params':{'edit':{'changes':{uri:edits(documents[uri]['text'],'command_name')}}}})
        answer=read()
        log('command_edit_result', result=answer.get('result'))
        send({'jsonrpc':'2.0','id':message['id'],'result':None})
    elif method == 'textDocument/didOpen':
        document = params['textDocument']
        documents[document['uri']] = document
        log('opened', uri=document['uri'], version=document['version'])
        if mode in ('push', 'unversioned', 'stale'):
            report = {'uri': document['uri'], 'diagnostics': []}
            if mode != 'unversioned':
                report['version'] = document['version'] + (1 if mode == 'stale' else 0)
            send({'jsonrpc': '2.0', 'method': 'textDocument/publishDiagnostics', 'params': report})
    elif method == 'textDocument/didClose':
        documents.pop(params['textDocument']['uri'], None)
    elif method == 'shutdown':
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': None})
    elif method == 'textDocument/diagnostic':
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': {'kind': 'full', 'items': ([{'message': 'Missing range'}] if mode == 'bad_diagnostic' else [])}})
    elif method == 'textDocument/documentSymbol':
        if mode == 'impact':
            text = documents[params['textDocument']['uri']]['text']
            symbols = []
            for match in re.finditer(r'\bfn\s+(\w+)', text):
                selected = {'start': point(text, match.start(1)), 'end': point(text, match.end(1))}
                symbols.append({'name': match.group(1), 'kind': 12, 'range': selected, 'selectionRange': selected})
            send({'jsonrpc':'2.0','id':message['id'],'result':symbols})
            continue
        if mode == 'bad_symbol':
            send({'jsonrpc': '2.0', 'id': message['id'], 'result': [{'name': 'invalid', 'kind': 13, 'range': {'start': {'line': 0, 'character': 0}, 'end': {'line': 0, 'character': 1}}, 'selectionRange': {'start': {'line': 9, 'character': 0}, 'end': {'line': 9, 'character': 1}}}]})
            continue
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': [{'name': 'old_name', 'kind': 13,
            'range': {'start': {'line': 0, 'character': 3}, 'end': {'line': 0, 'character': 11}},
            'selectionRange': {'start': {'line': 0, 'character': 3}, 'end': {'line': 0, 'character': 11}}}]})
    elif method in ('textDocument/definition', 'textDocument/implementation'):
        location = {'uri': params['textDocument']['uri'], 'range': {'start': params['position'], 'end': params['position']}}
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': location if method.endswith('definition') else [location]})
    elif method == 'textDocument/hover':
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': {'contents': {'kind': 'markdown', 'value': 'type: fixture'}}})
    elif method == 'textDocument/codeAction':
        document = documents[params['textDocument']['uri']]
        edit = {'changes': {document['uri']: edits(document['text'], 'action_name')}}
        actions = [{'title': 'Rename through code action', 'kind': 'refactor.rename', 'edit': edit},
                   {'title': 'Command action', 'command': {'command': 'unsafe.command', 'title': 'unsafe'}},
                   {'title': 'Deferred action', 'data': {'fixture': True}}]
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': actions})
    elif method == 'codeAction/resolve':
        document = next(iter(documents.values()))
        action = {**params, 'edit': {'changes': {document['uri']: edits(document['text'], 'resolved_name')}}}
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': action})
    elif method == 'textDocument/references':
        if mode == 'impact':
            text = documents[params['textDocument']['uri']]['text']
            line = text.splitlines()[params['position']['line']]
            name = re.match(r'\w+', line[params['position']['character']:]).group(0)
            locations = []
            for file in sorted(root.rglob('*.rs'))[:128]:
                source = file.read_text(encoding='utf-8')
                for match in re.finditer(r'\b' + re.escape(name) + r'\b', source):
                    locations.append({'uri':file.as_uri(),'range':{'start':point(source,match.start()),'end':point(source,match.end())}})
            send({'jsonrpc':'2.0','id':message['id'],'result':locations})
            continue
        if mode == 'hang':
            time.sleep(60)
            continue
        location = {'uri': params['textDocument']['uri'], 'range': {'start': params['position'], 'end': params['position']}}
        if mode == 'bad_reference':
            location['range']['end'] = {'line': -1, 'character': 0}
        external = {**location, 'uri': (root.parent / 'outside.txt').as_uri()}
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': [location, external]})
    elif mode == 'impact' and method == 'textDocument/prepareCallHierarchy':
        uri = params['textDocument']['uri']; text = documents[uri]['text']
        line = text.splitlines()[params['position']['line']]
        name = re.match(r'\w+', line[params['position']['character']:]).group(0)
        selected = {'start': params['position'], 'end': params['position']}
        item = {'name':name,'kind':12,'uri':uri,'range':selected,'selectionRange':selected,'data':name}
        send({'jsonrpc':'2.0','id':message['id'],'result':[item]})
    elif mode == 'impact' and method.startswith('callHierarchy/'):
        item = params['item']; result = []
        functions = []
        for file in sorted(root.rglob('*.rs'))[:128]:
            text = file.read_text(encoding='utf-8')
            for match in re.finditer(r'\bfn\s+(\w+)\s*\([^)]*\)\s*\{([^}]*)\}', text):
                selected = {'start':point(text,match.start(1)),'end':point(text,match.end(1))}
                functions.append(({'name':match.group(1),'kind':12,'uri':file.as_uri(),'range':selected,'selectionRange':selected,'data':match.group(1)},match.group(2)))
        if method.endswith('incomingCalls'):
            for caller, body in functions:
                if re.search(r'\b'+re.escape(item['name'])+r'\s*\(',body):
                    result.append({'from':caller,'fromRanges':[caller['selectionRange']]})
        else:
            body = next((body for own,body in functions if own['uri']==item['uri'] and own['name']==item['name']), '')
            for callee, _ in functions:
                if re.search(r'\b'+re.escape(callee['name'])+r'\s*\(',body):
                    result.append({'to':callee,'fromRanges':[item['selectionRange']]})
        send({'jsonrpc':'2.0','id':message['id'],'result':result})
    elif method == 'textDocument/prepareRename':
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': {'range': {'start': params['position'], 'end': params['position']}, 'placeholder': 'old_name'}})
    elif method == 'textDocument/rename':
        document = documents[params['textDocument']['uri']]
        changes = [{'textDocument': {'uri': document['uri'], 'version': document['version'] + (1 if mode == 'wrong_version' else 0)},
                    'edits': edits(document['text'], params['newName'])}]
        other = root / 'other.rs'
        if other.exists():
            changes.append({'textDocument': {'uri': other.as_uri(), 'version': None}, 'edits': edits(other.read_text(encoding='utf-8'), params['newName'])})
        if mode == 'outside':
            changes[0]['textDocument']['uri'] = (root.parent / 'outside.txt').as_uri()
        if mode == 'resource':
            changes = [{'kind': 'delete', 'uri': document['uri']}]
        send({'jsonrpc': '2.0', 'id': message['id'], 'result': {'documentChanges': changes}})
    elif 'id' in message:
        send({'jsonrpc': '2.0', 'id': message['id'], 'error': {'code': -32601, 'message': 'Fixture method not found'}})
