import { fullLibrary } from "./specification";
import { GENUI_LIMITS } from "./catalog";
import { validateProgram } from "./validation";

/** Bounded syntax-only repair. Reorders named component arguments into the
 * official positional contract; values, strings, object keys and facts stay
 * unchanged. No execution, invented defaults, missing-reference repair or LLM.
 */
export function repairProgram(source: string, streaming = false): {
  program: string;
  repaired: boolean;
} {
  if (source.length > GENUI_LIMITS.sourceChars)
    return { program: source, repaired: false };
  const trimmed = source
    .trim()
    .replace(/^```(?:openui-lang|openui)?\s*/, "")
    .replace(/```\s*$/, "");
  const schemas = fullLibrary.toJSONSchema().$defs ?? {};
  // A completed model response sometimes closes its parent array immediately
  // after an array-valued component argument, omitting that component's `)`.
  // Insert only this unambiguous closer; never append EOF content or values.
  function closeComponentBeforeArray(input: string): string {
    const stack: { kind: string; component: boolean }[] = [];
    let output = "", quote = "", escaped = false, insertions = 0;
    for (let i = 0; i < input.length; i++) {
      const char = input[i];
      if (quote) {
        output += char;
        if (escaped) escaped = false;
        else if (char === "\\") escaped = true;
        else if (char === quote) quote = "";
        continue;
      }
      if (char === '"' || char === "'") { quote = char; output += char; continue; }
      if ("([{".includes(char)) {
        if (stack.length >= GENUI_LIMITS.syntaxDepth) throw new Error("depth");
        const name = /([A-Za-z_]\w*)\s*$/.exec(input.slice(Math.max(0, i - 128), i))?.[1];
        stack.push({ kind: char, component: char === "(" && !!name && !!schemas[name] });
      } else if (")]}".includes(char)) {
        const top = stack[stack.length - 1];
        if (char === "]" && top?.kind === "(" && top.component && output.trimEnd().endsWith("]")) {
          if (++insertions > 4) throw new Error("repair bound");
          output += ")";
          stack.pop();
        }
        const expected = "([{"[")]}".indexOf(char)];
        if (stack.pop()?.kind !== expected) throw new Error("unmatched");
      }
      output += char;
    }
    if (quote || stack.length) throw new Error("incomplete");
    return output;
  }
  function transform(input: string, depth: number): string {
    if (depth > GENUI_LIMITS.syntaxDepth) throw new Error("depth");
    let output = "",
      i = 0;
    while (i < input.length) {
      const ch = input[i];
      if (ch === '"' || ch === "'") {
        const start = i++;
        while (i < input.length) {
          if (input[i] === "\\") {
            i += 2;
            continue;
          }
          if (input[i++] === ch) break;
        }
        output += input.slice(start, i);
        continue;
      }
      const name = /^[A-Za-z_]\w*/.exec(input.slice(i))?.[0];
      if (!name) {
        output += ch;
        i++;
        continue;
      }
      const start = i;
      i += name.length;
      while (/\s/.test(input[i] ?? "") && i < input.length) i++;
      if (input[i] !== "(" || !schemas[name]) {
        output += input.slice(start, i);
        continue;
      }
      const opening = i++;
      let nesting = 1,
        quote = "",
        escaped = false;
      for (; i < input.length && nesting; i++) {
        const c = input[i];
        if (quote) {
          if (escaped) escaped = false;
          else if (c === "\\") escaped = true;
          else if (c === quote) quote = "";
          continue;
        }
        if (c === '"' || c === "'") {
          quote = c;
          continue;
        }
        if (c === "(") nesting++;
        if (c === ")") nesting--;
        if (nesting > GENUI_LIMITS.syntaxDepth) throw new Error("depth");
      }
      if (nesting) throw new Error("incomplete");
      const inner = input.slice(opening + 1, i - 1);
      const args: string[] = [];
      let boundary = 0;
      const stack: string[] = [];
      quote = "";
      escaped = false;
      for (let j = 0; j < inner.length; j++) {
        const c = inner[j];
        if (quote) {
          if (escaped) escaped = false;
          else if (c === "\\") escaped = true;
          else if (c === quote) quote = "";
          continue;
        }
        if (c === '"' || c === "'") {
          quote = c;
          continue;
        }
        if ("([{".includes(c)) stack.push(c);
        if (")]}".includes(c)) stack.pop();
        if (c === "," && stack.length === 0) {
          args.push(inner.slice(boundary, j).trim());
          boundary = j + 1;
        }
      }
      if (inner.slice(boundary).trim()) args.push(inner.slice(boundary).trim());
      const named = args.map((arg) =>
        /^([A-Za-z_]\w*)\s*:\s*([\s\S]*)$/.exec(arg),
      );
      let repaired = args;
      if (named.some(Boolean)) {
        if (!named.every(Boolean)) throw new Error("mixed");
        const fields = Object.keys(schemas[name].properties ?? {});
        const values = new Map<string, string>();
        for (const item of named) {
          if (!item || !fields.includes(item[1]) || values.has(item[1]))
            throw new Error("unknown field");
          values.set(item[1], item[2]);
        }
        const last = Math.max(
          ...[...values.keys()].map((key) => fields.indexOf(key)),
        );
        repaired = fields.slice(0, last + 1).map((key) => {
          if (!values.has(key)) throw new Error("missing positional value");
          return values.get(key)!;
        });
      }
      const fields = Object.keys(schemas[name].properties ?? {});
      const normalized = repaired.map((arg, index) => {
        const converted = transform(arg, depth + 1);
        const property = (schemas[name].properties ?? {})[fields[index]] as
          | { type?: string }
          | undefined;
        // A singleton component/object where an array is required has one exact,
        // information-preserving correction. Final full validation still gates it.
        if (
          property?.type === "array" &&
          (/^[A-Za-z_]\w*\s*\(/.test(converted) || converted.startsWith("{"))
        )
          return "[" + converted + "]";
        return converted;
      });
      output += name + "(" + normalized.join(", ") + ")";
    }
    return output;
  }
  try {
    const program = transform(closeComponentBeforeArray(trimmed), 0);
    if (validateProgram(program, streaming).valid)
      return { program, repaired: program !== source };
  } catch {
    /* Keep original for the normal validated fallback. */
  }
  return { program: source, repaired: false };
}
