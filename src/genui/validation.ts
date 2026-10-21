import { autoClose, createParser, parseExpression, split, tokenize, type ASTNode } from '@openuidev/lang-core';
import { GENUI_LIMITS, GENUI_ROOT, genuiComponents } from './catalog';
import { fullLibrary } from './specification';

const parser = createParser(fullLibrary.toJSONSchema(), GENUI_ROOT);
const schemas = new Map(genuiComponents.map(component => [component.name, component.props]));

export type ProgramValidation = { valid: true } | { valid: false; diagnostic: string };
const invalid = (diagnostic: string): ProgramValidation => ({ valid: false, diagnostic });

/** Cheap lexical bounds run BEFORE the official parser can allocate/evaluate.
 * This display-only library deliberately has no reactive/function/tool syntax.
 * Quoted code samples remain literal and may contain any such tokens.
 */
function lexicalProblem(source: string): string | undefined {
  if (source.length > GENUI_LIMITS.sourceChars) return 'UI program exceeds 65,536 characters.';
  let quote = '', escaped = false;
  const delimiters: string[] = [];
  let structure = '';
  for (let i = 0; i < source.length; i++) {
    const char = source[i];
    if (quote) {
      if (escaped) escaped = false;
      else if (char === '\\') escaped = true;
      else if (char === quote) quote = '';
      continue;
    }
    if (char === '/') return 'Comments and arithmetic are unavailable in this library version.';
    if (char === '"' || char === "'") { quote = char; structure += ' '; continue; }
    if (char === '@' || char === '$') return 'Functions, actions and state expressions are unavailable in this library version.';
    if ('([{'.includes(char)) {
      delimiters.push(char);
      if (delimiters.length > GENUI_LIMITS.syntaxDepth) return 'UI syntax is nested too deeply.';
    }
    const closingIndex = ')]}'.indexOf(char);
    if (closingIndex >= 0 && delimiters.pop() !== '([{'[closingIndex]) return 'UI syntax has unmatched delimiters.';
    structure += char;
  }
  // Bound reference expansion before materialization. A short DAG of repeated
  // references could otherwise allocate an exponentially large tree.
  const assignments = [...structure.matchAll(/\b([A-Za-z_]\w*)\s*=(?!=)/g)];
  if (assignments.length > GENUI_LIMITS.statements) return 'Too many UI statements.';
  if (new Set(assignments.map(match => match[1])).size !== assignments.length) return 'Duplicate UI assignments are unavailable.';
  const dependencies = new Map<string, string[]>();
  // Use the upstream tokenizer, statement splitter and AST rather than a
  // second grammar for dependency analysis. No values are materialized here.
  let astNodes = 0;
  for (const statement of split(tokenize(autoClose(source).text))) {
    if (dependencies.has(statement.id)) return 'Duplicate UI assignments are unavailable.';
    const references: string[] = [];
    const pending: ASTNode[] = [parseExpression(statement.tokens)];
    while (pending.length) {
      if (++astNodes > 8_192) return 'Too many UI data values.';
      const node = pending.pop()!;
      switch (node.k) {
        case 'Comp':
          if (!schemas.has(node.name)) return 'Only catalog component calls are available.';
          pending.push(...node.args);
          break;
        case 'Ref': references.push(node.n); break;
        case 'Arr': pending.push(...node.els); break;
        case 'Obj': pending.push(...node.entries.map(([, value]) => value)); break;
        case 'Str': case 'Num': case 'Bool': case 'Null': case 'Ph': break;
        default: return 'Dynamic expressions are unavailable in this library version.';
      }
    }
    dependencies.set(statement.id, references);
  }
  // The upstream parser can fall back to another Response statement when root
  // is absent. Require the same entry we bound, rather than parsing a fallback.
  if (!dependencies.has('root')) return 'The UI program must declare root.';
  let expansions = 0;
  function visit(name: string, ancestors: Set<string>): boolean {
    if (++expansions > GENUI_LIMITS.nodes || ancestors.size > GENUI_LIMITS.depth || ancestors.has(name)) return false;
    const next = new Set(ancestors).add(name);
    return (dependencies.get(name) ?? []).every(child => !dependencies.has(child) || visit(child, next));
  }
  if (!visit('root', new Set())) return 'UI references are circular or expand beyond the resource limit.';
}

/** Zod validates each materialized component, because the official parser's
 * structural validation does not enforce every string/array/numeric bound.
 * Never use Zod's transformed .ref data as a replacement render tree.
 */
export function validateProgram(source: string, streaming = false): ProgramValidation {
  try {
    const problem = lexicalProblem(source);
    if (problem) return invalid(problem);
    const result = parser.parse(source);
    if (result.queryStatements.length || result.mutationStatements.length ||
        Object.keys(result.stateDeclarations).length || result.rootExpression) {
      return invalid('This presentation cannot run queries, mutations or dynamic expressions.');
    }
    if (result.meta.statementCount > GENUI_LIMITS.statements) return invalid('Too many UI statements.');
    if (!streaming && (result.meta.incomplete || result.meta.unresolved.length)) return invalid('The UI program is incomplete or has missing references.');
    if (result.meta.errors.some(error => error.severity !== 'warning')) return invalid('The UI program contains invalid components or properties.');
    if (!result.root) return streaming ? { valid: true } : invalid('The UI program has no Response root.');
    if (result.root.typeName !== GENUI_ROOT) return invalid('The UI root must be Response.');

    let nodes = 0;
    const sliders = new Map<string, Record<string, unknown>>();
    const bindings: string[] = [];
    const stateKeys = new Set<string>();
    const stack: { value: unknown; depth: number }[] = [{ value: result.root, depth: 0 }];
    while (stack.length) {
      const { value, depth } = stack.pop()!;
      if (depth > GENUI_LIMITS.depth) return invalid('The UI tree is nested too deeply.');
      if (!value || typeof value !== 'object') continue;
      if (Array.isArray(value)) {
        for (const child of value) stack.push({ value: child, depth: depth + 1 });
      } else {
        const record = value as Record<string, unknown>;
        if (record.type === 'element') {
          if (++nodes > GENUI_LIMITS.nodes) return invalid('Too many UI components.');
          const schema = schemas.get(String(record.typeName));
          if (!schema) return invalid('Unknown UI component.');
          if (record.hasDynamicProps) return invalid('Dynamic properties are unavailable in this library version.');
          const props = record.props as Record<string, unknown>;
          const name = String(record.typeName);
          if (name === "Slider") sliders.set(String(props.id), props);
          if (name === "FunctionPlot") bindings.push(String(props.parameterId));
          const id = String(props.id ?? '');
          const keys = ['Quiz','Flashcard','Slider','Toggle','Select','Input'].includes(name) ? [id] : name === 'Calculator' ? [`${id}-a`, `${id}-b`] : name === 'InteractiveExample' ? ['step','rate','join','speed'].map(suffix => `${id}-${suffix}`) : name === 'SourceCode' ? [`source-code-${props.fenceIndex}-search`,`source-code-${props.fenceIndex}-expanded`] : name === 'SourceChart' ? [`host-chart-${props.id}`] : name === 'Table' ? [String(props.id ?? `table-${(props.columns as string[] ?? []).join('-')}`)] : name === 'FileTree' ? [`file-tree-${props.id ?? 'default'}`] : name === 'DiffViewer' ? [`diff-${props.id ?? 'default'}-side-by-side`] : name === 'FilePreview' ? [`file-${props.path}-search`, `file-${props.path}-expanded`] : name === 'FunctionPlot' ? [`chart-${props.title}`] : name === 'SeriesChart' ? [`series-${props.title}`, ...(props.series as {name:string}[] ?? []).map(series => `chart-${props.title} · ${series.name}`)] : name === 'Chart' ? [`chart-${props.title}`] : [];
          for (const key of keys) {
            if (['constructor','prototype','__proto__'].includes(key)) return invalid('Reserved local state ID.');
            if (stateKeys.has(key)) return invalid('Duplicate local state IDs are unavailable.');
            stateKeys.add(key);
          }
          if (!schema.safeParse(record.props).success) return invalid(`Invalid or oversized ${String(record.typeName)} properties.`);
        }
        for (const child of Object.values(record)) stack.push({ value: child, depth: depth + 1 });
      }
    }
    if (bindings.some(id => !sliders.has(id))) return invalid("FunctionPlot requires a numeric Slider with the same parameterId.");
    return { valid: true };
  } catch {
    return invalid('The UI program could not be parsed safely.');
  }
}
