// Offline assertion of actual saved model output; never injects into the webview.
import { readFileSync } from 'node:fs';
import { runnerImport } from 'vite';
import { createParser } from '@openuidev/lang-core';
const record = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const { module: { repairProgram } } = await runnerImport('./src/genui/repair.ts', { configFile: false });
const { module: { validateProgram } } = await runnerImport('./src/genui/validation.ts', { configFile: false });
const { module: { validateChartGrounding } } = await runnerImport('./src/genui/dataGrounding.ts', { configFile: false });
const { module: { fullLibrary } } = await runnerImport('./src/genui/specification.ts', { configFile: false });
const {module:{numericTables,scatterCoordinate}}=await runnerImport('./src/genui/authoritativeData.ts',{configFile:false});
const tables=numericTables(record.sourceText);
const program = repairProgram(record.uiProgram).program;
if (record.format !== 'openui' || record.generationStatus !== 'complete' || !validateProgram(program).valid || !validateChartGrounding(program, record.sourceText)) throw new Error('Required model chart failed validation/grounding/status');
const root = createParser(fullLibrary.toJSONSchema(), 'Response').parse(program).root;
const charts = []; const pending = [root];
while (pending.length) {
  const node = pending.pop();
  if (!node || typeof node !== 'object') continue;
  if (node.typeName === 'Chart') charts.push(node.props);
  if (node.typeName === 'SourceChart' && tables[node.props.tableIndex]) charts.push({kind:node.props.kind,points:tables[node.props.tableIndex].points.map(point=>({...point,...(node.props.kind==='scatter'?{x:scatterCoordinate(point.label)}:{})}))});
  pending.push(...Object.values(node));
}
const expected = [{ label: 'January', value: 10 }, { label: 'February', value: 15 }, { label: 'March', value: 25 }];
if (charts.length !== 1 || charts[0].kind !== 'bar' || charts[0].points.length !== expected.length || !charts[0].points.every((point,i)=>point.label===expected[i].label&&point.value===expected[i].value&&point.x===undefined)) throw new Error('Model chart points did not exactly match required associations');
console.log(JSON.stringify({ valid: true, points: charts[0].points }));
