import { lazy, Suspense } from "react";
import type { Message } from "../lib/types";
import {
  MarkdownView,
  type MarkdownViewProps,
} from "../components/MarkdownView";
const VisualAnswer = lazy(() => import("./VisualAnswer"));
export function MessageRenderer({
  message,
  ...props
}: Omit<MarkdownViewProps, "content"> & { message: Message }) {
  const p = message.presentation;
  if (message.role === "assistant" && p?.format === "openui")
    return (
      <Suspense
        fallback={
          <MarkdownView {...props} assistant={false} content={p.sourceText} />
        }
      >
        <VisualAnswer
          key={message.id}
          record={p}
          conversationId={message.conversationId}
          {...props}
        />
      </Suspense>
    );
  return (
    <MarkdownView
      {...props}
      assistant={
        p?.format === "markdown" || message.status === "streaming"
          ? false
          : props.assistant
      }
      content={message.content}
    />
  );
}
