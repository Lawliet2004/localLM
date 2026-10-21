import type { ReactNode } from "react";
import type { ComponentRenderProps } from "@openuidev/react-lang";
import type { z } from "zod/v4";
import * as C from "./richCatalog";
import { adapt } from "./componentTypes";
import { FunctionPlot, EvidenceView, SeriesChart, SourceChart, SourceCode } from "./Experiences";
import { Chart } from "./Chart";
import { Diagram } from "./Diagram";
import { Quiz, Flashcard, Example, Control, Calculator } from "./Education";
import { FilePreview, SourceCard, DiffViewer, FileTree } from "./Coding";
export const richRenderers: Record<
  string,
  (p: ComponentRenderProps<Record<string, unknown>>) => ReactNode
> = {
  SourceCode: adapt(C.SourceCode.props, SourceCode),
  SourceChart: adapt(C.SourceChart.props, SourceChart),
  FunctionPlot: adapt(C.FunctionPlot.props, FunctionPlot),
  EvidenceView: adapt(C.EvidenceView.props, EvidenceView),
  SeriesChart: adapt(C.SeriesChart.props, SeriesChart),
  Calculator: adapt(C.Calculator.props,Calculator),
  Chart: adapt(C.Chart.props, Chart),
  Diagram: adapt(C.Diagram.props, Diagram),
  Quiz: adapt(C.Quiz.props, Quiz),
  Flashcard: adapt(C.Flashcard.props, Flashcard),
  InteractiveExample: adapt(C.InteractiveExample.props, Example),
  FilePreview: adapt(C.FilePreview.props, FilePreview),
  SourceCard: adapt(C.SourceCard.props, SourceCard),
  Slider: ({ props }) => <Control kind="Slider" props={props} />,
  Toggle: ({ props }) => <Control kind="Toggle" props={props} />,
  Select: ({ props }) => <Control kind="Select" props={props} />,
  Input: ({ props }) => <Control kind="Input" props={props} />,
  Timeline: ({ props }) => (
    <section>
      <h3>{String(props.title)}</h3>
      <ol className="genui-timeline">
        {(props.events as z.infer<typeof C.Timeline.props>["events"]).map(
          (e, i) => (
            <li key={i}>
              <time>{e.date}</time>
              <strong>{e.title}</strong>
              <p>{e.detail}</p>
            </li>
          ),
        )}
      </ol>
    </section>
  ),
  FileTree: adapt(C.FileTree.props, FileTree),
  DiffViewer: adapt(C.DiffViewer.props, DiffViewer),
};
