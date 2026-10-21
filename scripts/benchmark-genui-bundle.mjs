// Reproducible build experiment only. No production file is edited; no server.
import { build } from 'vite';
import { readFileSync,readdirSync,statSync,mkdirSync,writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { gzipSync } from 'node:zlib';
const root=resolve('.local/qa/genui-bundle-comparison');mkdirSync(root,{recursive:true});
const stubs={
 '/src/genui/MessageRenderer.tsx':`import {createElement} from 'react';import {MarkdownView} from '../components/MarkdownView';export function MessageRenderer({message,...props}){return createElement(MarkdownView,{...props,content:message.presentation?.sourceText??message.content});}`,
 '/src/components/GenUiBlock.tsx':`import {createElement} from 'react';import {CodeBlock} from './CodeBlock';export function GenUiBlock({code}){return createElement(CodeBlock,{code,language:'plaintext'});}`,
 '/src/genui/GenUiSurface.tsx':`import {createElement} from 'react';import {CodeBlock} from '../components/CodeBlock';export function GenUiSurface({code}){return createElement(CodeBlock,{code,language:'plaintext'});}`,
};
const measurements={};
for(const baseline of [true,false]){
 const name=baseline?'markdown':'visual',outDir=resolve(root,name);
 await build({configFile:resolve('vite.config.ts'),logLevel:'error',plugins:baseline?[{name:'benchmark-markdown-entrypoints',enforce:'pre',load(id){const key=Object.keys(stubs).find(key=>id.endsWith(key));return key?stubs[key]:null;}}]:[],build:{outDir,emptyOutDir:true}});
 const files=readdirSync(resolve(outDir,'assets')).filter(path=>/\.(?:js|css)$/.test(path));
 measurements[name]={bytes:0,gzipBytes:0,files:files.length};
 for(const path of files){const content=readFileSync(resolve(outDir,'assets',path));measurements[name].bytes+=statSync(resolve(outDir,'assets',path)).size;measurements[name].gzipBytes+=gzipSync(content).length;}
}
const report={measuredAt:new Date().toISOString(),measurements,incremental:{bytes:measurements.visual.bytes-measurements.markdown.bytes,gzipBytes:measurements.visual.gzipBytes-measurements.markdown.gzipBytes},method:'Same source/config and fonts; baseline substitutes Markdown/literal-code implementations at the three visual entrypoints. Sum of all JS/CSS, not network initial-load or lazy-load timing.'};
writeFileSync(resolve(root,'results.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report,null,2));
