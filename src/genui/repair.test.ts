import { describe,it,expect } from 'vitest';
import { repairProgram } from './repair';
import { validateProgram } from './validation';
describe('bounded syntax repair',()=>{
 it('repairs named arguments without changing chart data',()=>{const result=repairProgram('root = Response(children: [Chart(points: [{label:"Jan",value:10}], kind: "bar", title: "Sales")])');expect(result.repaired).toBe(true);expect(result.program).toContain('Chart("Sales", "bar", [{label:"Jan",value:10}])');expect(validateProgram(result.program).valid).toBe(true);});
 it('repairs one omitted component closer before a parent array without changing values',()=>{
 const source='root = Response([Chart(title:"Sales Data",kind:"bar",points:[{label:"January",value:10},{label:"February",value:15},{label:"March",value:25}]])';
 const result=repairProgram(source);expect(result.repaired).toBe(true);expect(result.program).toBe('root = Response([Chart("Sales Data", "bar", [{label:"January",value:10},{label:"February",value:15},{label:"March",value:25}])])');expect(validateProgram(result.program).valid).toBe(true);
 });
 it('does not append missing EOF structure or complete quoted values',()=>{
 for(const source of ['root = Response([Text("a")','root = Response([Text("unterminated','root = Response([Text("a")})'])expect(repairProgram(source)).toEqual({program:source,repaired:false});
 });
 it('keeps literal code and object keys unchanged',()=>{const source='root = Response([CodeBlock(code: "Text(text: \\\"literal\\\")", language: "text")])';const result=repairProgram(source);expect(result.program).toContain('Text(text:');expect(validateProgram(result.program).valid).toBe(true);});
 it('wraps schema-declared singleton arrays without inventing values',()=>{
 const result=repairProgram('root = Response(Chart(title:"Sales",kind:"bar",points:{label:"Jan",value:10}))');expect(result.repaired).toBe(true);expect(validateProgram(result.program).valid).toBe(true);
 });
 it.each(['root = Response([Unknown(text:"bad")])','root = Response([Text(value:"bad")])','root = Response([Text("a",text:"b")])','root = Response([Text(text:"unterminated'])('does not guess missing or invalid structure: %s',source=>{expect(repairProgram(source)).toEqual({program:source,repaired:false});});
});
