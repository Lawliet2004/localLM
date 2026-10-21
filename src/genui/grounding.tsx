import { createContext, useContext } from "react";
import { safeSourceUrl } from "../lib/sources";
export const EvidenceContext = createContext<Record<string, string>>({});
// null retains legacy fenced-block behavior; new presentation messages always
// provide an explicit set derived from authoritative content/evidence.
export const AllowedUrlContext = createContext<Set<string> | null>(null);
export function useEvidence() {
  return useContext(EvidenceContext);
}
export function useAllowedUrls() {
  return useContext(AllowedUrlContext);
}
export function parseEvidence(raw: string): Record<string, string> {
  try {
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return Object.fromEntries(
      Object.entries(value)
        .filter(([, url]) => typeof url === "string" && url.length <= 2048)
        .slice(0, 256),
    );
  } catch {
    return {};
  }
}
export function groundedUrls(
  text: string,
  evidence: Record<string, string> | null | undefined,
): Set<string> {
  const candidates = [
    ...Object.values(evidence ?? {}),
    ...(text.match(/https:\/\/[^\s<>"')\]]+/g) ?? []),
  ];
  const result = new Set<string>();
  for (const value of candidates) {
    const url = safeSourceUrl(
      typeof value === "string" ? value.replace(/[.,;!?]+$/, "") : "",
    );
    if (url?.protocol === "https:" && !url.username && !url.password)
      result.add(url.href);
  }
  return result;
}
export function isGroundedUrl(value: string, allowed: Set<string>): boolean {
  const url = safeSourceUrl(value);
  return !!url && allowed.has(url.href);
}

export interface HostClaim { claim: string; verdict: string; citations: string[] }
export const ResearchContext = createContext<{ sources: {url:string;title?:string;snippet?:string;date?:string;ids:string[]}[]; claims: HostClaim[] }>({sources:[],claims:[]});

export const AuthoritativeDataContext = createContext<import("./authoritativeData").NumericTable[]>([]);

export const AuthoritativeCodeContext = createContext<import("./authoritativeData").CodeSample[]>([]);
