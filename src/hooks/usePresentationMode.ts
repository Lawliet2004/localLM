import { useEffect, useRef, useState } from "react";
import { api } from "../lib/api";
import type { PresentationMode, PresentationModes } from "../lib/types";
export const PRESENTATION_DEFAULT_CHANGED = "locallm:presentation-default-changed";
// Serialize global preference writes across modal instances. Successful writes
// notify mounted chats even if the initiating Settings modal has closed.
let defaultWrite: Promise<void> = Promise.resolve();
export function savePresentationDefault(mode: PresentationMode): Promise<void> {
  const write = defaultWrite.catch(() => {}).then(() => api.setPresentationMode(null, mode));
  defaultWrite = write;
  return write.then(() => window.dispatchEvent(new CustomEvent(PRESENTATION_DEFAULT_CHANGED, { detail: mode }))).then(() => {});
}
const defaults: PresentationModes = {
  defaultMode: "auto",
  overrideMode: null,
  effectiveMode: "auto",
};
// Read-only: presentation preferences are edited in Settings, never the composer.
export function usePresentationMode(conversation: string) {
  const [settings, setSettings] = useState<PresentationModes>(defaults);
  const latestDefault = useRef<PresentationMode | null>(null);
  const owner = useRef(conversation),
    revision = useRef(0);
  owner.current = conversation;
  useEffect(() => {
    let active = true;
    const token = ++revision.current;
    setSettings(defaults);
    if (api.getPresentationMode)
      void api
        .getPresentationMode(conversation === "new" ? null : conversation)
        .then((value) => {
          if (
            active &&
            value &&
            owner.current === conversation &&
            revision.current === token
          )
            setSettings({ ...value, defaultMode: latestDefault.current ?? value.defaultMode, effectiveMode: value.overrideMode ?? latestDefault.current ?? value.defaultMode });
        })
        .catch(() => {});
    return () => {
      active = false;
    };
  }, [conversation]);
  useEffect(() => {
    const update = (event: Event) => {
      const mode = (event as CustomEvent<PresentationMode>).detail;
      latestDefault.current = mode;
      setSettings(current => ({ ...current, defaultMode: mode, effectiveMode: current.overrideMode ?? mode }));
    };
    window.addEventListener(PRESENTATION_DEFAULT_CHANGED, update);
    return () => window.removeEventListener(PRESENTATION_DEFAULT_CHANGED, update);
  }, []);
  return settings;
}
