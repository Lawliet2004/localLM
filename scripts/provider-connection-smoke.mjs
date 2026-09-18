import { createServer } from 'node:http';
import assert from 'node:assert/strict';

// Exercise the real desktop IPC, encrypted vault, HTTP adapter and persistence.
let modelRequests = 0;
const server = createServer((request, response) => {
  assert.equal(request.headers.authorization, 'Bearer locallm-smoke-fixture');
  if (request.url === '/custom/models' && modelRequests++ === 0) {
    response.writeHead(200, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ data: [{ id: 'fixture-model', description: 'x'.repeat(20000) }] }));
  } else if (request.url === '/custom/chat/completions') {
    let body = '';
    request.on('data', chunk => { body += chunk; });
    request.on('end', () => {
      const payload = JSON.parse(body);
      assert.equal(payload.model, 'fixture-model');
      assert.ok(payload.messages.some(message => message.role === 'user' && message.content.includes('attachment-fixture-42')));
      response.writeHead(200, { 'Content-Type': 'text/event-stream' });
      response.end('data: ' + JSON.stringify({choices:[{index:0,delta:{content:'Attachment received.'},finish_reason:null}]}) + '\n\ndata: ' + JSON.stringify({choices:[{index:0,delta:{},finish_reason:'stop'}]}) + '\n\ndata: [DONE]\n\n');
    });
  } else { response.writeHead(401); response.end('{}'); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const targets = await (await fetch('http://127.0.0.1:9223/json/list')).json();
const socket = new WebSocket(targets.find(target => target.type === 'page').webSocketDebuggerUrl);
await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }));
try {
  const result = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Native smoke timed out')), 30000);
    socket.addEventListener('message', event => {
      const message = JSON.parse(event.data);
      if (message.id === 1) { clearTimeout(timer); resolve(message.result); }
    });
    socket.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { awaitPromise: true, returnByValue: true,
      expression: `(async () => {
        const {invoke, Channel} = await import('/node_modules/@tauri-apps/api/core.js');
        let provider, conversation;
        try {
          provider = await invoke('save_provider', {draft:{name:'Connection smoke fixture', apiFormat:'openai-chat-completions',baseUrl:'http://127.0.0.1:${server.address().port}/custom',apiKey:'locallm-smoke-fixture',models:[{id:'fixture-model',contextLength:2000000,maxOutputTokens:100000,toolSupport:'unsupported'}]}});
          const test = await invoke('test_provider', {id:provider.id});
          const saved = (await invoke('list_providers')).find(item=>item.id===provider.id);
          conversation = await invoke('create_conversation');
          await invoke('save_conversation_model',{id:conversation.id,selection:{providerId:provider.id,modelId:'fixture-model'}});
          const {composeMessage} = await import('/src/lib/attachments.ts');
          const content = composeMessage('Read this file', [{id:'fixture',name:'note.txt',size:21,content:'attachment-fixture-42'}]);
          const channel = new Channel(); channel.onmessage = () => {};
          await invoke('send_message', {conversationId:conversation.id,content,connectorIds:[],connectorTools:[],channel});
          const messages = await invoke('get_messages',{id:conversation.id});
          const reply = messages.find(message=>message.role==='assistant');
          const attachmentPersisted = messages.some(message=>message.role==='user' && message.content===content);
          let rejected = false;
          try { await invoke('test_provider', {id:provider.id}); } catch { rejected = true; }
          const afterFailure = (await invoke('list_providers')).find(item=>item.id===provider.id);
          return {reply:reply.content, replyStatus:reply.status, attachmentPersisted, rejected, verifiedAfterFailure:afterFailure.verified, verified:test.verified, models:test.models, savedVerified:saved.verified, hasApiKey:saved.hasApiKey, keyExposed:JSON.stringify(saved).includes('locallm-smoke-fixture')};
        } finally { if(conversation) await invoke('delete_conversation',{id:conversation.id}); if(provider) await invoke('delete_provider',{id:provider.id}); }
      })()` } }));
  });
  assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
  assert.deepEqual(result.result.value, { reply: 'Attachment received.', replyStatus: 'complete', attachmentPersisted: true, rejected: true, verifiedAfterFailure: false, verified: true, models: ['fixture-model'], savedVerified: true, hasApiKey: true, keyExposed: false });
  console.log('Native provider smoke passed: save, encrypted credential, custom endpoint, large catalog, verification persistence, failed re-test invalidation, attachment inference and persistence, credential redaction and fixture cleanup.');
} finally { socket.close(); server.close(); }
