import { useCallback, useMemo, useEffect, useRef, useState } from "react";
import type { PresentationRecord } from "../lib/types";
import { api } from "../lib/api";
import {
  MarkdownView,
  type MarkdownViewProps,
} from "../components/MarkdownView";
import { ValidatedRenderer } from "./ValidatedRenderer";
import { validateChartGrounding } from './dataGrounding';
import { progressiveProgram } from "./progressive";
import { repairProgram } from "./repair";
import { validateProgram } from "./validation";
import { GENUI_LIBRARY_VERSION } from "./catalog";
import { presentationAction } from "./actions";
import {
  EvidenceContext,
  AuthoritativeDataContext,
  AuthoritativeCodeContext,
  AllowedUrlContext,
  groundedUrls,
  isGroundedUrl,
  parseEvidence,
} from "./grounding";
import { numericTables, codeSamples } from "./authoritativeData";
import { nativeAvailable } from "../lib/api";
import { openUrl } from "@tauri-apps/plugin-opener";
export default function VisualAnswer({
  record: p,
  conversationId,
  ...props
}: Omit<MarkdownViewProps, "content"> & {
  record: PresentationRecord;
  conversationId: string;
}) {
  const [text, setText] = useState(false),
    [error, setError] = useState("");
  const [renderFailed, setRenderFailed] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const pending = useRef<Record<string, unknown> | undefined>(undefined);
  const persist = useCallback(() => {
    if (pending.current) {
      const next = pending.current;
      pending.current = undefined;
      void api
        .saveVisualState(conversationId, p.messageId, next)
        .catch(() => setError("Interactive state could not be saved."));
    }
  }, [conversationId, p.messageId]);
  useEffect(
    () => () => {
      clearTimeout(timer.current);
      persist();
    },
    [persist],
  );
  const streaming = p.generationStatus === "streaming" || !!props.streaming;
  const compatible =
    p.libraryVersion === GENUI_LIBRARY_VERSION && p.formatVersion === 1;
  const prepared = useMemo(
    () =>
      p.generationStatus === "complete"
        ? repairProgram(p.uiProgram)
        : { program: p.uiProgram, repaired: false },
    [p.uiProgram, p.generationStatus],
  );
  const committed = useMemo(() => streaming ? progressiveProgram(prepared.program, program => validateChartGrounding(program,p.sourceText,parseEvidence(props.sourceLinks ?? "{}"))) : prepared.program, [streaming,prepared.program,p.sourceText,props.sourceLinks]);
  const validation = compatible
    ? validateProgram(committed ?? "", streaming)
    : { valid: false };
  const grounded = useMemo(()=>!committed || !validation.valid || validateChartGrounding(committed,p.sourceText,parseEvidence(props.sourceLinks ?? "{}")),[committed,validation.valid,p.sourceText,props.sourceLinks]);
  useEffect(() => {
    if (compatible && p.generationStatus === "complete" && (!validation.valid || !grounded || renderFailed))
      void api
        .rejectPresentation(conversationId, p.messageId, p.uiProgram)
        .catch(() => {});
  }, [
    compatible,
    p.generationStatus,
    p.messageId,
    p.uiProgram,
    conversationId,
    validation.valid,grounded,renderFailed,
  ]);
  const failed =
    renderFailed ||
    !compatible ||
    p.generationStatus === "error" ||
    p.generationStatus === "interrupted" ||
    (!streaming && (!validation.valid || !grounded || renderFailed));
  const codes = useMemo(() => codeSamples(p.sourceText), [p.sourceText]);
  const tables = useMemo(() => numericTables(p.sourceText), [p.sourceText]);
  const evidence = parseEvidence(props.sourceLinks ?? "{}");
  const allowedUrls = groundedUrls(p.sourceText, evidence);
  return (
    <section className="genui-answer" aria-label="Visual answer">
      <div className="genui-controls">
        <button
          type="button"
          className="genui-btn"
          onClick={() => setText((v) => !v)}
          aria-pressed={text}
        >
          {text ? "Visual answer" : "Original answer"}
        </button>
        {!streaming && (
          <details>
            <summary>UI source</summary>
            <pre>{p.uiProgram}</pre>
          </details>
        )}
      </div>
      {text || failed ? (
        <>
          <MarkdownView {...props} assistant={false} content={p.sourceText} />
          {failed && (
            <p className="genui-muted" role="status">
              {p.diagnostic ||
                "Visual output was invalid or incompatible. The original answer is preserved."}
            </p>
          )}
        </>
      ) : (
        <EvidenceContext.Provider value={evidence}>
          <AllowedUrlContext.Provider value={allowedUrls}><AuthoritativeDataContext.Provider value={tables}><AuthoritativeCodeContext.Provider value={codes}>
            <ValidatedRenderer
              response={grounded ? committed ?? "" : ""}
              onError={(errors) => {
                if (!errors.some(error => error.severity !== "warning")) return;
                setRenderFailed(true);
                setError("Visual rendering failed. The original answer is preserved.");
                void api.rejectPresentation(conversationId, p.messageId, p.uiProgram).catch(() => {});
              }}
              onRenderError={() => {
                setRenderFailed(true);
                setError("Visual rendering failed. The original answer is preserved.");
                void api.rejectPresentation(conversationId, p.messageId, p.uiProgram).catch(() => {});
              }}
            fallback={<MarkdownView {...props} assistant={false} content={p.sourceText}/>}
              isStreaming={streaming}
              initialState={p.interactiveState}
              onStateUpdate={(state) => {
                if (streaming) return;
                pending.current = state;
                clearTimeout(timer.current);
                timer.current = setTimeout(persist, 250);
              }}
              onAction={(event) => {
                if (streaming) return;
                const action = presentationAction(event);
                if (!action) return;
                if (action.type === "continue_conversation") {
                  props.onSendPrompt?.(action.message);
                } else if (!isGroundedUrl(action.url, allowedUrls)) {
                  setError(
                    "That link is absent from the original answer and evidence.",
                  );
                } else if (nativeAvailable) {
                  void openUrl(action.url).catch(() =>
                    setError("Could not open the link."),
                  );
                } else {
                  window.open(action.url, "_blank", "noopener,noreferrer");
                }
              }}
            />
          </AuthoritativeCodeContext.Provider></AuthoritativeDataContext.Provider></AllowedUrlContext.Provider>
        </EvidenceContext.Provider>
      )}
      {prepared.repaired && !failed && (
        <p className="genui-muted">
          Component argument syntax corrected locally.
        </p>
      )}
      {error && <p role="status">{error}</p>}
    </section>
  );
}
