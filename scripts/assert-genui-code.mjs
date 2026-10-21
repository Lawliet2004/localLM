import { readFileSync } from 'node:fs';
import { runnerImport } from 'vite';
import { createParser } from '@openuidev/lang-core';
const record=JSON.parse(readFileSync(process.argv[2],'utf8')),code=readFileSync(process.argv[3],'utf8');
const {module:{repairProgram}}=await runnerImport('./src/genui/repair.ts',{configFile:false});
const {module:{validateProgram}}=await runnerImport('./src/genui/validation.ts',{configFile:false});
const {module:{validateChartGrounding}}=await runnerImport('./src/genui/dataGrounding.ts',{configFile:false});
const {module:{fullLibrary}}=await runnerImport('./src/genui/specification.ts',{configFile:false});
const {module:{codeSamples}}=await runnerImport("./src/genui/authoritativeData.ts",{configFile:false});
const samples=codeSamples(record.sourceText);
const program=repairProgram(record.uiProgram).program;
if(record.format!=='openui'||record.generationStatus!=='complete'||!validateProgram(program).valid||!validateChartGrounding(program,record.sourceText))throw new Error('Tool-assisted presentation did not complete with valid grounded UI');
const pending=[createParser(fullLibrary.toJSONSchema(),'Response').parse(program).root];let found=false;
while(pending.length){const node=pending.pop();if(!node||typeof node!=='object')continue;if(['FilePreview','CodeBlock'].includes(node.typeName)&&node.props.code===code)found=true;if(node.typeName==='SourceCode'&&samples[node.props.fenceIndex]?.code===code)found=true;pending.push(...Object.values(node));}
if(!found)throw new Error('Tool-assisted presentation did not preserve the exact file code');
console.log(JSON.stringify({valid:true,exactCodePreserved:true}));
