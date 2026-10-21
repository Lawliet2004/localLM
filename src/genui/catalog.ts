// Generative UI component catalog — the single source of truth for what local
// and cloud agents may render inside ```openui fenced blocks (OpenUI Lang,
// https://github.com/thesysdev/openui). This file is intentionally React-free:
// src/genui/library.tsx attaches renderers, and scripts/build-genui-prompt.mjs
// generates the model instructions from the same definitions, so the prompt
// can never drift from what the app actually renders.
//
// Keep the catalog small and deterministic. Props are positional in OpenUI
// Lang, so required fields must come first in each schema.

import { defineComponent } from "@openuidev/lang-core";
import { z } from "zod/v4";
import { richComponents, richGroups } from "./richCatalog";

export const GENUI_ROOT = "Response";
export const GENUI_LIBRARY_VERSION = "locallm.openui/2";

// Bounds are enforced locally as well as serialized into the library schema.
export const GENUI_LIMITS = {
  sourceChars: 65_536,
  textChars: 16_384,
  labelChars: 240,
  children: 32,
  rows: 100,
  columns: 12,
  statements: 80,
  nodes: 128,
  depth: 16,
  syntaxDepth: 32,
} as const;

const text = () => z.string().max(GENUI_LIMITS.textChars);
const label = () => z.string().max(GENUI_LIMITS.labelChars);

// lang-core `defineComponent` is renderer-agnostic; renderers live in
// library.tsx. `null` satisfies the generic `component` slot for spec use.
function spec<T extends z.core.$ZodObject>(config: {
  name: string;
  description: string;
  props: T;
}) {
  return defineComponent({ ...config, component: null });
}

// ── Content ────────────────────────────────────────────────────────────────

export const Heading = spec({
  name: 'Heading', description: 'A section heading.',
  props: z.object({ text: label(), level: z.enum(['h2', 'h3', 'h4']).optional() }),
});

export const Text = spec({
  name: 'Text', description: 'Plain text, no Markdown.',
  props: z.object({ text: text() }),
});

export const Markdown = spec({
  name: "Markdown",
  description:
    "A block of Markdown text. Use for prose, explanations and formatted text inside the UI.",
  props: z.object({
    text: text().describe("Markdown-formatted content"),
  }),
});

export const Callout = spec({
  name: "Callout",
  description:
    "A highlighted note for a warning, tip, success or error the user should notice.",
  props: z.object({
    text: text().describe("Short message, plain text"),
    tone: z
      .enum(["info", "success", "warning", "error"])
      .optional()
      .describe("Visual tone; defaults to info"),
  }),
});

export const CodeBlock = spec({
  name: "CodeBlock",
  description: "A labeled block of source code or terminal output.",
  props: z.object({
    code: text().describe("Raw code or output"),
    language: z.string().max(40).optional().describe("Language tag, e.g. python, bash"),
  }),
});

// ── Data display ───────────────────────────────────────────────────────────

export const Stat = spec({
  name: "Stat",
  description: "A single metric with a label and an optional detail line.",
  props: z.object({
    label: label().describe("Metric name"),
    value: label().describe("Metric value"),
    detail: label().optional().describe("Supporting detail"),
  }),
});

export const Table = spec({
  name: "Table",
  description:
    "A simple data table. Use instead of a Markdown table when the data is the focus.",
  props: z.object({
    columns: z.array(label()).min(1).max(GENUI_LIMITS.columns).describe("Column headers, at most 12"),
    rows: z
      .array(z.array(z.union([label(), z.number().finite()])).max(GENUI_LIMITS.columns))
      .max(GENUI_LIMITS.rows).describe("At most 100 rows; each row matches columns"),
    id: z.string().max(64).optional(),
  }).refine(value => value.rows.every(row => row.length === value.columns.length), {
    message: "Every row must have the same number of cells as column headers",
  }),
});

export const List = spec({
  name: "List",
  description: "A short bulleted list of plain-text items.",
  props: z.object({
    items: z.array(text()).max(32).describe("At most 32 bullet items"),
  }),
});

export const KeyValue = spec({
  name: "KeyValue",
  description: "Label/value pairs such as settings, status fields or metadata.",
  props: z.object({
    entries: z
      .array(z.object({ label: label(), value: label() })).max(32)
      .describe("Pairs rendered as label: value rows"),
  }),
});

export const Progress = spec({
  name: "Progress",
  description: "A progress bar for completion, usage or quota values.",
  props: z.object({
    label: label().describe("What is being measured"),
    percent: z.number().min(0).max(100).describe("Completion from 0 to 100"),
    detail: label().optional().describe("Small line under the bar"),
  }),
});

// ── Actions ────────────────────────────────────────────────────────────────

export const buttonAction = z
  .union([
    z
      .object({
        type: z.literal("continue_conversation"),
        message: z
          .string()
          .min(1).max(4_000)
          .optional()
          .describe("Message sent on click; defaults to the button label"),
      })
      .describe("Send the user a suggested follow-up message"),
    z
      .object({
        type: z.literal("open_url"),
        url: z.string().max(2_048).url().refine(value => {
          try {
            const url = new URL(value);
            return url.protocol === 'https:' && !url.username && !url.password;
          } catch { return false; }
        }).describe("https URL without credentials"),
      })
      .describe("Open a link in the system browser"),
  ])
  .optional();

export const Button = spec({
  name: "Button",
  description:
    "A clickable button for suggested follow-ups or opening a link. Place one or a small row at the end of a UI block.",
  props: z.object({
    label: label().min(1).describe("Button text"),
    action: buttonAction.describe(
      "What clicking does; omit to send the label as a follow-up message"
    ),
    variant: z
      .enum(["primary", "secondary"])
      .optional()
      .describe("Emphasis; defaults to secondary"),
  }),
});

// ── Layout ─────────────────────────────────────────────────────────────────

export const Divider = spec({
  name: "Divider",
  description: "A thin horizontal rule separating sections.",
  props: z.object({}),
});

const leaf = z.union([
  ...richComponents.map(component => component.ref),
  Heading.ref,
  Text.ref,
  Markdown.ref,
  Callout.ref,
  CodeBlock.ref,
  Stat.ref,
  Table.ref,
  List.ref,
  KeyValue.ref,
  Progress.ref,
  Button.ref,
  Divider.ref,
]);

export const Section = spec({
  name: "Section",
  description: "A titled panel grouping related content.",
  props: z.object({
    title: label().describe("Section heading"),
    children: z.array(leaf).max(GENUI_LIMITS.children).describe("At most 32 children"),
  }),
});

export const Columns = spec({
  name: "Columns",
  description:
    "Places children side by side, wrapping on narrow widths. Ideal for a row of Stat cards.",
  props: z.object({
    children: z.array(leaf).max(12).describe("At most 12 items, wrapping horizontally"),
  }),
});

export const Card = spec({
  name: 'Card', description: 'A titled group of related content.',
  props: z.object({ title: label(), children: z.array(leaf).max(GENUI_LIMITS.children) }),
});

export const Stack = spec({
  name: 'Stack', description: 'Content arranged vertically.',
  props: z.object({ children: z.array(z.union([leaf, Card.ref])).max(GENUI_LIMITS.children) }),
});

export const Row = spec({
  name: 'Row', description: 'Content in a wrapping horizontal row.',
  props: z.object({ children: z.array(z.union([leaf, Card.ref])).max(12) }),
});

export const Grid = spec({
  name: 'Grid', description: 'Responsive equal-width content columns.',
  props: z.object({
    children: z.array(z.union([leaf, Card.ref])).max(12),
    columns: z.number().int().min(1).max(4).optional(),
  }),
});

export const Response = spec({
  name: "Response",
  description:
    "Root container of every UI block. Assign it to `root` and pass content as children.",
  props: z.object({
    children: z
      .array(z.union([...leaf.options, Section.ref, Columns.ref, Card.ref, Stack.ref, Row.ref, Grid.ref]))
      .max(GENUI_LIMITS.children).describe("At most 32 top-level children"),
  }),
});

export const genuiComponents = [
  ...richComponents,
  Heading,
  Text,
  Markdown,
  Callout,
  CodeBlock,
  Stat,
  Table,
  List,
  KeyValue,
  Progress,
  Button,
  Divider,
  Section,
  Columns,
  Card,
  Stack,
  Row,
  Grid,
  Response,
];

export const genuiComponentGroups = [
  ...richGroups,
  { name: "Content", components: ["Heading", "Text", "Markdown", "Callout", "CodeBlock"] },
  { name: "Data", components: ["Stat", "Table", "List", "KeyValue", "Progress"] },
  { name: "Actions", components: ["Button"] },
  { name: "Layout", components: ["Divider", "Section", "Columns", "Card", "Stack", "Row", "Grid", "Response"] },
];
