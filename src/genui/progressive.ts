import { GENUI_LIMITS } from './catalog';
import { repairProgram } from './repair';
import { validateProgram } from './validation';

/** Commit only syntactically closed component calls. Closures apply to outer
 * containers, never strings or child data. Incomplete children are quarantined.
 * Named arguments are normalized only after the whole child has arrived.
 */
export function progressiveProgram(source: string, accept: (program: string) => boolean = () => true): string | undefined {
  if (source.length > GENUI_LIMITS.sourceChars) return;
  let quote = '', escaped = false;
  const stack: string[] = [], candidates: string[] = [];
  for (let i = 0; i < source.length; i++) {
    const c = source[i];
    if (quote) {
      if (escaped) escaped = false;
      else if (c === '\\') escaped = true;
      else if (c === quote) quote = '';
      continue;
    }
    if (c === '"' || c === "'") { quote = c; continue; }
    if ('([{'.includes(c)) {
      stack.push(c);
      if (stack.length > GENUI_LIMITS.syntaxDepth) return;
    } else if (')]}'.includes(c)) {
      if (stack.pop() !== '([{'[')]}'.indexOf(c)]) return;
      if (c === ')' && candidates.length < 256) {
        candidates.push(source.slice(0, i + 1) + [...stack].reverse().map(open => ')]}'['([{'.indexOf(open)]).join(''));
      }
    }
  }
  for (const candidate of candidates.reverse()) {
    const repaired = repairProgram(candidate, true).program;
    if (validateProgram(repaired, true).valid && accept(repaired)) return repaired;
  }
}
