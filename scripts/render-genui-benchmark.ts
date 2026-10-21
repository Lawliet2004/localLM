// Offline React render proof only. No web server, browser or generated execution.
import {createElement} from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {Renderer} from '@openuidev/react-lang';
import {locallmLibrary} from '../src/genui/library';
import {AuthoritativeDataContext,AuthoritativeCodeContext} from '../src/genui/grounding';
import {numericTables,codeSamples} from '../src/genui/authoritativeData';
export function renderBenchmarkProgram(program:string,answer:string):string {
 return renderToStaticMarkup(createElement(AuthoritativeCodeContext.Provider,{value:codeSamples(answer)},createElement(AuthoritativeDataContext.Provider,{value:numericTables(answer)},createElement(Renderer,{response:program,library:locallmLibrary,toolProvider:null,publishObservability:false}))));
}
