// React renderers for the generative UI catalog (src/genui/catalog.ts).
// Visuals follow the app's design tokens; all class names are genui-*.

import { createLibrary, defineComponent, useFormName, useIsStreaming, useTriggerAction, type ComponentRenderProps } from '@openuidev/react-lang';
import { Info, CircleCheck, TriangleAlert, CircleX } from 'lucide-react';
import { type ReactNode } from 'react';
import * as catalog from './catalog';
import { buttonAction } from './catalog';
import { CodeBlock } from '../components/CodeBlock';
import { VisualMarkdown } from './VisualMarkdown';
import './genui.css';
import { useAllowedUrls,isGroundedUrl } from './grounding';
import { DataTable } from './DataTable';
import { richRenderers } from './RichComponents';

type RenderProps = ComponentRenderProps<Record<string, unknown>>;

function nodes(props: Record<string, unknown>, renderNode: RenderProps['renderNode']): ReactNode {
  return renderNode(props.children as never[]);
}

const calloutIcons: Record<string, ReactNode> = {
  info: <Info size={15} aria-hidden="true" />,
  success: <CircleCheck size={15} aria-hidden="true" />,
  warning: <TriangleAlert size={15} aria-hidden="true" />,
  error: <CircleX size={15} aria-hidden="true" />,
};

const renderers: Record<string, (input: RenderProps) => ReactNode> = {
  ...richRenderers,
  Heading: ({ props }) => {
    const Tag = props.level === 'h2' ? 'h2' : props.level === 'h4' ? 'h4' : 'h3';
    return <Tag className="genui-heading">{String(props.text ?? '')}</Tag>;
  },
  Text: ({ props }) => <p className="genui-text">{String(props.text ?? '')}</p>,
  Markdown: ({ props }) => <VisualMarkdown text={String(props.text ?? '')} />,
  Callout: ({ props }) => {
    const tone = typeof props.tone === 'string' ? props.tone : 'info';
    return (
      <div className={`genui-callout genui-callout-${tone}`} role={tone === 'error' || tone === 'warning' ? 'alert' : 'note'}>
        <span className="genui-callout-icon">{calloutIcons[tone] ?? calloutIcons.info}</span>
        <span>{String(props.text ?? '')}</span>
      </div>
    );
  },
  CodeBlock: ({ props }) => <CodeBlock code={String(props.code ?? '')} language={typeof props.language === 'string' ? props.language : undefined} />,
  Stat: ({ props }) => (
    <div className="genui-stat">
      <span className="genui-stat-label">{String(props.label ?? '')}</span>
      <strong className="genui-stat-value">{String(props.value ?? '')}</strong>
      {props.detail != null && <small className="genui-stat-detail">{String(props.detail)}</small>}
    </div>
  ),
  Table: ({ props }) => {const parsed=catalog.Table.props.safeParse(props);return parsed.success?<DataTable props={parsed.data}/>:null;},
  List: ({ props }) => {
    const items = Array.isArray(props.items) ? props.items : [];
    return <ul className="genui-list">{items.map((item, i) => <li key={i}>{String(item)}</li>)}</ul>;
  },
  KeyValue: ({ props }) => {
    const entries = Array.isArray(props.entries) ? props.entries : [];
    return (
      <dl className="genui-kv">
        {entries.map((entry, i) => {
          const pair = entry && typeof entry === 'object' ? (entry as Record<string, unknown>) : {};
          return (
            <div className="genui-kv-row" key={i}>
              <dt>{String(pair.label ?? '')}</dt>
              <dd>{String(pair.value ?? '')}</dd>
            </div>
          );
        })}
      </dl>
    );
  },
  Progress: ({ props }) => {
    const percent = Math.max(0, Math.min(100, Number(props.percent) || 0));
    return (
      <div className="genui-progress">
        <div className="genui-progress-head">
          <span>{String(props.label ?? '')}</span>
          <span>{Math.round(percent)}%</span>
        </div>
        <div className="genui-progress-track" role="progressbar" aria-valuenow={percent} aria-valuemin={0} aria-valuemax={100} aria-label={String(props.label ?? 'Progress')}>
          <div className="genui-progress-fill" style={{ width: `${percent}%` }} />
        </div>
        {props.detail != null && <small className="genui-progress-detail">{String(props.detail)}</small>}
      </div>
    );
  },
  Button: ({ props }) => <GenuiButton props={props} />,
  Divider: () => <hr className="genui-divider" />,
  Section: ({ props, renderNode }) => (
    <section className="genui-section">
      <h3 className="genui-section-title">{String(props.title ?? '')}</h3>
      <div className="genui-section-body">{nodes(props, renderNode)}</div>
    </section>
  ),
  Columns: ({ props, renderNode }) => <div className="genui-columns">{nodes(props, renderNode)}</div>,
  Card: ({ props, renderNode }) => <section className="genui-section">
    <h3 className="genui-section-title">{String(props.title ?? '')}</h3>
    <div className="genui-section-body">{nodes(props, renderNode)}</div>
  </section>,
  Stack: ({ props, renderNode }) => <div className="genui-response">{nodes(props, renderNode)}</div>,
  Row: ({ props, renderNode }) => <div className="genui-columns">{nodes(props, renderNode)}</div>,
  Grid: ({ props, renderNode }) => <div className={`genui-grid genui-grid-${props.columns ?? 2}`}>{nodes(props, renderNode)}</div>,
  Response: ({ props, renderNode }) => <div className="genui-response">{nodes(props, renderNode)}</div>,
};

function GenuiButton({ props }: { props: Record<string, unknown> }) {
  const triggerAction = useTriggerAction();
  const formName = useFormName();
  const isStreaming = useIsStreaming();
  const allowed = useAllowedUrls();
  const parsed = buttonAction.safeParse(props.action);
  const action = parsed.success ? parsed.data : undefined;
  const variant = props.variant === 'primary' ? 'primary' : 'secondary';
  const label = String(props.label ?? '');
  return (
    <button
      type="button"
      className={`genui-btn genui-btn-${variant}`}
      disabled={isStreaming || (action?.type === 'open_url' && allowed !== null && !isGroundedUrl(action.url,allowed))}
      onClick={() => {
        const type = action?.type ?? 'continue_conversation';
        const params = action?.type === 'open_url'
          ? { url: action.url }
          : action?.type === 'continue_conversation' && action.message
          ? { message: action.message }
          : undefined;
        triggerAction(label, formName, { type, params });
      }}
    >
      {label}
    </button>
  );
}

export const locallmLibrary = createLibrary({
  id: catalog.GENUI_LIBRARY_VERSION,
  root: catalog.GENUI_ROOT,
  componentGroups: catalog.genuiComponentGroups,
  components: catalog.genuiComponents.map(spec =>
    defineComponent({
      name: spec.name,
      description: spec.description,
      props: spec.props,
      component: input => spec.props.safeParse(input.props).success
        ? renderers[spec.name](input) : null,
    })
  ),
});
