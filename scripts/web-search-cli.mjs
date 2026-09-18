#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { readFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
const args = process.argv.slice(2);
if (!args.length || args.includes('--help')) {
  console.log('LocalLM research: node scripts/web-search-cli.mjs ask "question" [--research fast|normal|deep] [--trace] [--sources] [--json] [--config path.json]\nHealth: node scripts/web-search-cli.mjs health\nSet LOCAL_LLM_BASE_URL and LOCAL_LLM_MODEL to use an already-running local model.');
} else {
  let mode='normal', config={}, trace=false, json=false, sources=false, enabled=true;
  const words=[];
  for(let i=['ask','search','health'].includes(args[0])?1:0;i<args.length;i++) {
    const arg=args[i];
    if(arg==='--research') mode=args[++i];
    else if(arg==='--config') config=JSON.parse(readFileSync(args[++i],'utf8'));
    else if(arg==='--trace') trace=true;
    else if(arg==='--json') json=true;
    else if(arg==='--sources') sources=true;
    else if(arg==='--no-web') enabled=false;
    else if(arg!=='--web' && arg.startsWith('--')) throw new Error(`Unknown option ${arg}`);
    else if(arg!=='--web') words.push(arg);
  }
  if(!['fast','normal','deep'].includes(mode)) throw new Error('Invalid research mode');
  const question=words.join(' ');
  if(!question && args[0]!=='health') throw new Error('Provide a research question');
  mkdirSync('.local',{recursive:true});
  const child=spawn(process.execPath,[resolve('src-tauri/resources/web/worker.mjs')],{stdio:['pipe','pipe','inherit'],windowsHide:true});
  child.stdin.end(JSON.stringify({question,mode,trace,full:json,action:args[0]==='health'?'health':undefined,
    config:{...config,enabled},databasePath:resolve('.local/web-research.sqlite'),
    localModel:process.env.LOCAL_LLM_BASE_URL?{baseUrl:process.env.LOCAL_LLM_BASE_URL,modelName:process.env.LOCAL_LLM_MODEL}:undefined}));
  let output='';
  for await(const chunk of child.stdout) output+=chunk;
  const code=await new Promise(resolve=>child.on('close',resolve));
  if(code!==0) process.exitCode=1;
  else {
    const result=JSON.parse(output);
    console.log(json || args[0]==='health'?JSON.stringify(result,null,2):result.answer);
    if(sources && !json) for(const source of Object.values(result.sources)) console.log(`[${source.id}] ${source.title}: ${source.url}`);
  }
}
