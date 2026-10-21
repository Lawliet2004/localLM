export const nativeAvailable = true;
export const errorMessage = error => error instanceof Error ? error.message : String(error);
let workspace = '/fixture/project';
let version = 1;
const entries = new Map([
  ['src', { kind: 'directory' }],
  ['src/main.ts', { kind: 'file', content: '// original\r\nconst answer = 42;\r\n' + '// line\r\n'.repeat(650), hash: 'initial' }],
  ['README.md', { kind: 'file', content: '# Workspace\n', hash: 'readme' }],
]);
const read = path => { const entry = entries.get(path); if (!entry || entry.kind !== 'file') throw Error('File not found.'); return entry; };
const browsers = new Map();
const browserCalls = [];
window.__workspaceFixture = {
  content: path => read(path).content,
  paths: () => [...entries.keys()],
  externalEdit: (path, content) => { entries.set(path, {kind:'file',content,hash:'external-'+ ++version}); },
  browserDelay: 0,
  browserState: () => ({ views: [...browsers].map(([label, view]) => ({label, ...view})), calls: browserCalls }),
};
export const api = {
  getWorkspace: async () => ({path:workspace}),
  setWorkspace: async path => {workspace=path;},
  workspaceInspect: async path => {
    const prefix = path === '.' ? '' : path + '/';
    return {entries:[...entries].filter(([name]) => name.startsWith(prefix) && !name.slice(prefix.length).includes('/')).map(([name, entry]) => ({name:name.slice(prefix.length),kind:entry.kind})),truncated:false};
  },
  workspaceSearch: async query => ({results:[...entries].filter(([path, entry])=>entry.kind==='file' && path.includes(query)).map(([path])=>({path,kind:'file'})),truncated:false}),
  editorRead: async (_root, path) => ({content:read(path).content,sha256:read(path).hash}),
  editorSave: async (_root,path,content,expected) => {
    if(read(path).hash!==expected) throw Error('This file changed on disk. Your edits are preserved. Reload the file to review the current version before saving.');
    const entry={kind:'file',content,hash:'saved-'+ ++version}; entries.set(path,entry);return {content,sha256:entry.hash};
  },
  editorMutate: async (_root,mutation) => {
    if(mutation.kind.startsWith('create')) {
      if(entries.has(mutation.path)) throw Error('A file or folder with that name already exists.');
      entries.set(mutation.path,mutation.kind==='createFile'?{kind:'file',content:'',hash:'new-'+ ++version}:{kind:'directory'});return mutation.path;
    }
    if(mutation.kind==='delete') { for(const path of [...entries.keys()]) if(path===mutation.path || path.startsWith(mutation.path+'/')) entries.delete(path); return mutation.path; }
    const target=mutation.path.split('/').slice(0,-1).concat(mutation.name).join('/');
    if(entries.has(target)) throw Error('Destination already exists.');
    for(const [path,entry] of [...entries]) if(path===mutation.path || path.startsWith(mutation.path+'/')) {entries.delete(path);entries.set(target+path.slice(mutation.path.length),entry);}
    return target;
  },
  workspaceGit: async () => ({repository:true,branch:'main',branches:['main','dev'],status:' M src/main.ts',files:[{path:'src/main.ts',status:' M'}],diff:'diff --git a/src/main.ts b/src/main.ts\n@@ -1 +1 @@\n-old\n+new\n'}),
  browserUrl: async label => ({url:browsers.get(label)?.url ?? 'about:blank'}),
  browserShow: async (label, ...bounds) => {
    if(window.__workspaceFixture.browserDelay) await new Promise(resolve => setTimeout(resolve, window.__workspaceFixture.browserDelay));
    browsers.set(label, {...browsers.get(label), visible:true, bounds});
    browserCalls.push({kind:'show',label,bounds});
  },
  browserHide: async label => { const view=browsers.get(label); if(view) view.visible=false; browserCalls.push({kind:'hide',label}); },
  browserClose: async label => { browsers.delete(label); browserCalls.push({kind:'close',label}); },
  browserNavigate: async (label,url) => { browsers.get(label).url=url; browserCalls.push({kind:'navigate',label}); },
};
