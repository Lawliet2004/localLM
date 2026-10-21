import { defineComponent } from "@openuidev/lang-core";
import { z } from "zod/v4";
const label = z.string().max(240);
const id = z.string().regex(/^[a-zA-Z][a-zA-Z0-9_-]{0,63}$/).refine(value => !["constructor", "prototype", "__proto__"].includes(value));
const value = z.number().finite().min(-1e12).max(1e12);
const text = z.string().max(16_384);
const spec = <T extends z.core.$ZodObject>(
  name: string,
  description: string,
  props: T,
) => defineComponent({ name, description, props, component: null });
export const Chart = spec(
  "Chart",
  "Chart of supplied values; never invent data.",
  z
    .object({
      title: label,
      kind: z.enum(["bar", "line", "pie", "donut", "scatter"]),
      points: z
        .array(z.object({ label, value, x: value.optional() }))
        .min(1)
        .max(200),
      xLabel: label.optional(),
      yLabel: label.optional(),
    })
    .refine(
      (v) =>
        !["pie", "donut"].includes(v.kind) ||
        (v.points.every((p) => p.value >= 0) &&
          v.points.some((p) => p.value > 0)),
      { message: "Pie values must be nonnegative with a positive total" },
    ),
);
export const SourceChart = spec('SourceChart', 'Chart exact host-bound values from a numeric two-column table in the authoritative answer; never transcribe or invent values.', z.object({
  id, tableIndex: z.number().int().min(0).max(7), kind:z.enum(['bar','line','pie','donut','scatter']),
}));
export const Diagram = spec(
  "Diagram",
  "Directed concept/process graph.",
  z
    .object({
      title: label,
      nodes: z
        .array(z.object({ id, label, detail: label.optional() }))
        .min(1)
        .max(24),
      edges: z
        .array(z.object({ from: id, to: id, label: label.optional() }))
        .max(48),
    })
    .refine(
      (v) =>
        new Set(v.nodes.map((n) => n.id)).size === v.nodes.length &&
        v.edges.every(
          (e) =>
            v.nodes.some((n) => n.id === e.from) &&
            v.nodes.some((n) => n.id === e.to),
        ),
    ),
);
export const Timeline = spec(
  "Timeline",
  "Ordered dated events.",
  z.object({
    title: label,
    events: z
      .array(z.object({ date: label, title: label, detail: label.optional() }))
      .max(32),
  }),
);
export const Quiz = spec(
  "Quiz",
  "Local scored multiple-choice quiz.",
  z
    .object({
      id,
      title: label,
      questions: z
        .array(
          z.object({
            question: label,
            options: z.array(label).min(2).max(6),
            answer: z.number().int().min(0).max(5),
            explanation: label,
          }),
        )
        .min(1)
        .max(12),
    })
    .refine((v) => v.questions.every((q) => q.answer < q.options.length)),
);
export const Flashcard = spec(
  "Flashcard",
  "Flip to reveal an answer.",
  z.object({ id, front: text, back: text }),
);
export const InteractiveExample = spec(
  "InteractiveExample",
  "Deterministic educational simulation, not execution.",
  z.object({
    id,
    kind: z.enum([
      "sorting",
      "binary-search",
      "lru",
      "bfs",
      "dfs",
      "sql-join",
      "gradient-descent",
    ]),
    values: z.array(z.number().int().min(-999).max(999)).min(1).max(24),
    target: z.number().int().min(-999).max(999).optional(),
    capacity: z.number().int().min(1).max(8).optional(),
    edges: z
      .array(
        z.tuple([
          z.number().int().min(0).max(23),
          z.number().int().min(0).max(23),
        ]),
      )
      .max(48)
      .optional(),
  }),
);
export const Slider = spec(
  "Slider",
  "Local numeric control.",
  z
    .object({
      id,
      label,
      min: value,
      max: value,
      value,
      step: z.number().min(0.001).max(100).optional(),
    })
    .refine((v) => v.min < v.max && v.value >= v.min && v.value <= v.max),
);
export const Toggle = spec(
  "Toggle",
  "Local on/off control.",
  z.object({ id, label, value: z.boolean() }),
);
export const Select = spec(
  "Select",
  "Local choice control.",
  z
    .object({ id, label, options: z.array(label).min(1).max(24), value: label })
    .refine((v) => v.options.includes(v.value)),
);
export const Input = spec(
  "Input",
  "Local text field. Never sends hidden requests.",
  z.object({ id, label, value: z.string().max(2000) }),
);
export const SourceCode = spec('SourceCode','Read-only exact code from a closed fence in the authoritative answer. Never transcribe, edit or execute it.',z.object({fenceIndex:z.number().int().min(0).max(15),path:label.optional()}));
export const FilePreview = spec(
  "FilePreview",
  "Literal code sample with display-only path.",
  z.object({
    path: label,
    code: text,
    language: z.string().max(40).optional(),
  }),
);
export const FileTree = spec(
  "FileTree",
  "Display-only file list; no filesystem access.",
  z.object({ paths: z.array(label).max(100), id: id.optional() }),
);
export const DiffViewer = spec(
  "DiffViewer",
  "Literal unified diff.",
  z.object({ diff: text, id: id.optional() }),
);
export const SourceCard = spec(
  "SourceCard",
  "Reference an actual evidence ID from the answer. Never invent IDs.",
  z.object({
    citationId: z.string().regex(/^[A-Za-z0-9_-]{1,64}$/),
    title: label,
    summary: label.optional(),
  }),
);

export const Calculator = spec('Calculator','Local bounded arithmetic; never executes code.',z.object({
 id,title:label,operation:z.enum(['add','multiply','divide','power']),a:value,b:value,
}));


export const FunctionPlot = spec('FunctionPlot', 'Educational finite function plot driven by a Slider with the same parameterId. No expressions or code.', z.object({
  title: label, parameterId: id, family: z.enum(['linear','quadratic','exponential']),
  minX: z.number().min(-10).max(10), maxX: z.number().min(-10).max(10),
}).refine(p => p.minX < p.maxX));
export const EvidenceView = spec('EvidenceView', 'Actual host-collected evidence and claim audit. IDs must resolve; generated claims are never accepted as audit metadata.', z.object({ citationIds: z.array(z.string().regex(/^[A-Za-z0-9_-]{1,64}$/)).min(1).max(32) }));
export const SeriesChart = spec('SeriesChart', 'Compare supplied series locally; each series can be hidden. Values must be supplied together with their series and label.', z.object({
  title: label, series: z.array(z.object({name: label.refine(value => !['__proto__','constructor','prototype'].includes(value)), points: z.array(z.object({label, value})).min(1).max(100)})).min(1).max(4),
}).refine(p => new Set(p.series.map(s => s.name)).size === p.series.length));

export const richComponents = [
  FunctionPlot, EvidenceView, SeriesChart, SourceChart, SourceCode,
  Calculator,
  Chart,
  Diagram,
  Timeline,
  Quiz,
  Flashcard,
  InteractiveExample,
  Slider,
  Toggle,
  Select,
  Input,
  FilePreview,
  FileTree,
  DiffViewer,
  SourceCard,
];
export const richGroups = [
  {
    name: "Visual data",
    components: ["Chart", "SourceChart", "SeriesChart", "Diagram", "Timeline", "SourceCard", "EvidenceView"],
  },
  {
    name: "Education",
    components: [
      "FunctionPlot",
      "Calculator",
      "Quiz",
      "Flashcard",
      "InteractiveExample",
      "Slider",
      "Toggle",
      "Select",
      "Input",
    ],
  },
  { name: "Coding", components: ["FilePreview", "SourceCode", "FileTree", "DiffViewer"] },
];
