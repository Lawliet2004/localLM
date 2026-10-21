import { describe,expect,it } from 'vitest';
import { validateChartGrounding } from './dataGrounding';
describe('factual chart boundary',()=>{
 it('accepts only supplied chart labels and values',()=>{const chart='root = Response([Chart("Sales","bar",[{label:"Jan",value:10}])])';expect(validateChartGrounding(chart,'Jan sales: 10')).toBe(true);expect(validateChartGrounding(chart,'Jan sales: 99')).toBe(false);expect(validateChartGrounding(chart,'Feb sales: 10')).toBe(false);});
 it('does not pretend educational values are observed measurements',()=>{expect(validateChartGrounding('root = Response([InteractiveExample("sort","sorting",[3,1,2])])','Sorting example')).toBe(true);});
});

it('binds table metrics to their headers and preserves typed Markdown column associations',()=>{
 const wrong='root = Response([Table(["Month","Revenue"],[["Jan",10]])])';
 expect(validateChartGrounding(wrong,'Jan sales: 10; Revenue: 20.')).toBe(false);
 expect(validateChartGrounding(wrong,'Jan sales: 10; Jan revenue: 20.')).toBe(false);
 expect(validateChartGrounding('root = Response([Table(["Month","Revenue"],[["Jan",20]])])','| Month | Revenue |\n| --- | --- |\n| Jan | 20 |')).toBe(true);
 expect(validateChartGrounding(wrong,'| Month | Revenue |\n| --- | --- |\n| Jan | 20 |')).toBe(false);
});

it('does not exchange citation associations within the same clause',()=>{
 const links={s1:'https://one.example',s2:'https://two.example'},source='Claim A [s1], Claim B [s2].';
 expect(validateChartGrounding('root = Response([SourceCard("s1","Claim A")])',source,links)).toBe(true);
 expect(validateChartGrounding('root = Response([SourceCard("s1","Claim B")])',source,links)).toBe(false);
});
