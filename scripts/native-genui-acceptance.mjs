import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, statSync, readdirSync } from 'node:fs';
import { mkdir,open,writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline';
const output=resolve(process.env.LOCALLM_SMOKE_OUTPUT||'.local/qa/genui-native');await mkdir(output,{recursive:true});
const sourcePaths=['src','src-tauri/src'];
const extraPaths=['src-tauri/Cargo.toml','src-tauri/Cargo.lock','package-lock.json','scripts/native-genui-acceptance.py','scripts/native-genui-acceptance.mjs','scripts/assert-genui-chart.mjs','scripts/assert-genui-code.mjs'];
function sourceFiles(path){return readdirSync(path,{withFileTypes:true}).flatMap(entry=>entry.isDirectory()?sourceFiles(`${path}/${entry.name}`):[`${path}/${entry.name}`]);}
const files = process.env.LOCALLM_SOURCE_SNAPSHOT==='1' ? [...sourcePaths.flatMap(sourceFiles),...extraPaths] : execFileSync('git', ['ls-files', '-co', '--exclude-standard', ...sourcePaths,...extraPaths], {encoding:'utf8'}).trim().split('\n').filter(Boolean);
const source = createHash('sha256');
for (const path of [...new Set(files)].sort()) { source.update(path); source.update(readFileSync(path)); }
const binary = resolve(process.env.LOCALLM_NATIVE_BINARY || 'src-tauri/target/debug/locallm');
if (files.filter(path=>(path.startsWith("src/")||path.startsWith("src-tauri/"))&&!/(?:\.test\.tsx?$|_tests\.rs$|test_support\.rs$)/.test(path)).some(path => statSync(path).mtimeMs > statSync(binary).mtimeMs)) throw new Error('Native binary predates source; rebuild before acceptance');
const sourceSha256=source.digest('hex');
const binarySha256=createHash('sha256').update(readFileSync(binary)).digest('hex');
await writeFile(resolve(output,'run-provenance.json'), JSON.stringify({startedAt:new Date().toISOString(),sourceSha256,binarySha256,sourceSnapshot:process.env.LOCALLM_SOURCE_SNAPSHOT==='1',platform:process.platform,arch:process.arch,node:process.version,scenario:'native deterministic controls plus optional selected local model',modelConfigured:!!process.env.LOCALLM_MODEL_PREFS_DB},null,2));
const xvfb=process.env.LOCALLM_XVFB_BIN||'Xvfb';
const display=spawn(xvfb,['-displayfd','1','-screen','0','1440x1000x24','-nolisten','tcp','-ac','-noreset'],{stdio:['ignore','pipe','inherit']});
let app;const log=await open(resolve(output,'native-app.log'),'w');
try{
 const number=await new Promise((done,fail)=>{const timeout=setTimeout(()=>fail(new Error('Xvfb timeout')),10000);const lines=createInterface({input:display.stdout});display.once('error',fail);lines.once('line',line=>{clearTimeout(timeout);lines.close();/^\d+$/.test(line)?done(line):fail(new Error('Invalid display'));});});
 const env={...process.env,DISPLAY:`:${number}`,WAYLAND_DISPLAY:'',GDK_BACKEND:'x11',XDG_SESSION_TYPE:'x11',XDG_DATA_HOME:resolve(output,'data'),XDG_CONFIG_HOME:resolve(output,'config'),XDG_CACHE_HOME:resolve(output,'cache'),LOCALLM_DESKTOP_TEST_ISOLATED:'1',NO_AT_BRIDGE:'0',GTK_MODULES:'gail:atk-bridge',LIBGL_ALWAYS_SOFTWARE:'1',WEBKIT_DISABLE_COMPOSITING_MODE:'1'};
 app=spawn('dbus-run-session',['--','/usr/bin/python3',resolve('scripts/native-genui-acceptance.py'),binary,output],{env,detached:true,stdio:['ignore',log.fd,log.fd]});
 const code=await new Promise((done,fail)=>{const timeout=setTimeout(()=>fail(new Error('Native acceptance timeout')),900000);app.once('exit',code=>{clearTimeout(timeout);done(code);});app.once('error',fail);});
 const finalSource=createHash('sha256');
 for(const path of [...new Set(files)].sort()){finalSource.update(path);finalSource.update(readFileSync(path));}
 if(createHash('sha256').update(readFileSync(binary)).digest('hex')!==binarySha256)throw new Error('Native binary changed during acceptance');
 if(finalSource.digest('hex')!==sourceSha256)throw new Error('Source changed during native acceptance; results do not certify current source');
 if(code!==0)throw new Error(`Native acceptance failed (${code}); inspect ${output}/native-app.log`);
}finally{if(app?.pid){try{process.kill(-app.pid,'SIGTERM');}catch{}}display.kill('SIGTERM');await log.close();}
