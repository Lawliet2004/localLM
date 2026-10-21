"""A paginated local MCP tool catalog; no network, dependencies or user data."""
import json, sys
log_path = sys.argv[1]
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request.get('method')
    params = request.get('params', {})
    if method == 'initialize':
        result = {'protocolVersion': params['protocolVersion'], 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'discovery-fixture', 'version': '1'}}
    elif method == 'tools/list':
        offset = int(params.get('cursor') or 0)
        result = {'tools': [{'name': 'fixture_%03d' % n, 'description': 'Selected fixture tool %d' % n,
                            'inputSchema': {'type': 'object', 'properties': {'value': {'type': 'string'}}, 'required': ['value'], 'additionalProperties': False}}
                           for n in range(offset, min(offset + 35, 70))]}
        if offset + 35 < 70:
            result['nextCursor'] = str(offset + 35)
    elif method == 'tools/call':
        with open(log_path, 'a', encoding='utf-8') as out:
            out.write(json.dumps(params) + '\n')
        content = {'tool': params['name'], 'value': params['arguments']['value'], 'processed': True}
        result = {'content': [{'type': 'text', 'text': json.dumps(content)}], 'structuredContent': content, 'isError': False}
    else:
        print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32601, 'message': 'Unsupported fixture method'}}), flush=True)
        continue
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
