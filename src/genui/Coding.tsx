import * as C from "./richCatalog";
import type { Props } from "./componentTypes";
import { useStateField, useIsStreaming } from "@openuidev/react-lang";
import { alignedDiff } from "./diff";
import { DiffLines } from "../components/DiffView";
import { CodeBlock } from "../components/CodeBlock";
import { safeSourceUrl } from "../lib/sources";
import { useEvidence } from "./grounding";
export function FilePreview({ props: p, stateId }: Props<typeof C.FilePreview.props> & {stateId?:string}) {
  const streaming = useIsStreaming();
  const searchField = useStateField(`${stateId??`file-${p.path}`}-search`, ""), expandedField = useStateField(`${stateId??`file-${p.path}`}-expanded`,true);
  const search=String(searchField.value??''), expanded=!!expandedField.value;
  const setSearch=searchField.setValue, setExpanded=expandedField.setValue;
  const lines = p.code.split("\n");
  return (
    <section>
      <h4>Code sample · {p.path}</h4>
      <label>
        Search code{" "}
        <input
          disabled={streaming}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          maxLength={240}
        />
      </label>
      <button
        type="button"
        className="genui-btn"
        disabled={streaming}
        onClick={() => setExpanded(!expanded)}
        aria-expanded={expanded}
      >
        {expanded ? "Collapse" : "Expand"}
      </button>
      {expanded && (
        <>
          <CodeBlock code={p.code} language={p.language} lineNumbers />
          {search && (
            <ol className="genui-code-matches">
              {lines.flatMap((line, i) =>
                line.includes(search)
                  ? [
                      <li key={i}>
                        Line {i + 1}: {line}
                      </li>,
                    ]
                  : [],
              )}
            </ol>
          )}
        </>
      )}
    </section>
  );
}
export function SourceCard({ props: p }: Props<typeof C.SourceCard.props>) {
  const sources = useEvidence();
  const url = safeSourceUrl(sources[p.citationId]);
  return (
    <aside className="genui-source">
      {url ? (
        <a href={url.href} target="_blank" rel="noopener noreferrer">
          [{p.citationId}] {p.title}
        </a>
      ) : (
        <span>Unverified source reference [{p.citationId}]</span>
      )}
      {p.summary && url && <p>{p.summary}</p>}
    </aside>
  );
}

export function DiffViewer({ props: p }: Props<typeof C.DiffViewer.props>) {
  const streaming = useIsStreaming();
  const sideField = useStateField(`diff-${p.id ?? "default"}-side-by-side`, false);
  const side = !!sideField.value;
  const rows = alignedDiff(p.diff);
  return (
    <section>
      <button
        type="button"
        className="genui-btn"
        aria-pressed={side}
        disabled={streaming}
        onClick={() => sideField.setValue(!side)}
      >
        {side ? "Unified diff" : "Side by side"}
      </button>
      {side ? (
        <div className="genui-table-wrap"><table className="genui-table genui-split-diff"><thead><tr><th>Old line</th><th>Before</th><th>New line</th><th>After</th></tr></thead><tbody>
          {rows.map((row, i) => row.heading ? <tr key={i}><th colSpan={4}>{row.heading}</th></tr> : <tr key={i}><td>{row.oldLine ?? ""}</td><td className={row.before !== row.after ? "genui-diff-removed" : ""}><code>{row.before ?? ""}</code></td><td>{row.newLine ?? ""}</td><td className={row.before !== row.after ? "genui-diff-added" : ""}><code>{row.after ?? ""}</code></td></tr>)}
        </tbody></table></div>
      ) : (
        <DiffLines diff={p.diff} />
      )}
    </section>
  );
}

interface TreeNode { name: string; children: Map<string, TreeNode>; file: boolean }
export function FileTree({ props: p }: Props<typeof C.FileTree.props>) {
  const expansion = useStateField<Record<string, boolean>>(`file-tree-${p.id ?? "default"}`, {});
  const streaming = useIsStreaming();
  const root: TreeNode = {name:"",children:new Map(),file:false};
  for (const path of p.paths) {
    let node = root;
    for (const part of path.split(/[\\/]/).filter(Boolean).slice(0,24)) {
      if (!node.children.has(part)) node.children.set(part,{name:part,children:new Map(),file:false});
      node = node.children.get(part)!;
    }
    node.file = true;
  }
  const render = (node: TreeNode, parent = ""): React.ReactNode => <ul className="genui-file-tree">{[...node.children.values()].sort((a,b)=>Number(!a.children.size)-Number(!b.children.size)||a.name.localeCompare(b.name)).map(child=><li key={child.name}>{child.children.size ? <details open={expansion.value?.[`${parent}/${child.name}`] !== false}><summary onClick={event => { event.preventDefault(); if (!streaming) expansion.setValue({...expansion.value,[`${parent}/${child.name}`]:expansion.value?.[`${parent}/${child.name}`] === false}); }}>{child.name}/</summary>{render(child,`${parent}/${child.name}`)}</details> : <code>{child.name}</code>}</li>)}</ul>;
  return <nav aria-label="Read-only file hierarchy">{render(root)}</nav>;
}
