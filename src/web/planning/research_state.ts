/**
 * Research State: structured Plan → search → read → record → gaps →
 * investigate → verify → answer loop state.
 *
 * The engine keeps this state across retry rounds: original requirements and
 * constraints, open research questions, findings with supporting passages,
 * contradictions, rejected candidates, completed/pending actions, and next
 * actions. Repeated actions and lack of new evidence end the loop instead of
 * burning more budget.
 */

import type { EvidenceClaim, SourceRegistry } from '../types';

export interface ResearchRequirement {
  id: string;
  text: string;
  status: 'answered' | 'unresolved' | 'blocked';
  reason?: string;
}

export interface ResearchFinding {
  requirementId: string;
  claim: string;
  supportingPassages: string[];
  sources: string[];
  confidence: number;
}

export interface ResearchState {
  question: string;
  constraints: string[];
  requirements: ResearchRequirement[];
  openQuestions: string[];
  findings: ResearchFinding[];
  contradictions: Array<{ claims: string[]; sources: string[] }>;
  rejected: Array<{ candidate: string; reason: string }>;
  completedActions: string[];
  pendingActions: string[];
  evidenceHashes: string[];
  roundsWithoutNewEvidence: number;
}

export function initResearchState(question: string): ResearchState {
  const requirements = splitRequirements(question);
  return {
    question,
    constraints: [],
    requirements: requirements.map((text, i) => ({ id: `REQ-${i + 1}`, text, status: 'unresolved' as const })),
    openQuestions: [...requirements],
    findings: [],
    contradictions: [],
    rejected: [],
    completedActions: [],
    pendingActions: requirements.map((text) => `Gather evidence: ${text}`),
    evidenceHashes: [],
    roundsWithoutNewEvidence: 0,
  };
}

export function splitRequirements(question: string): string[] {
  const parts = question
    .split(/\b(?:and|versus|\bvs\b|compared (?:with|to)|;|\?)/i)
    .map((s) => s.replace(/^(compare|contrast|research|find|what (is|are)|which)\b/i, '').trim())
    .filter((s) => s.length > 3);
  if (parts.length <= 1) return [question.trim()];
  return parts.slice(0, 5);
}

export function recordRound(
  state: ResearchState,
  action: string,
  claims: EvidenceClaim[],
  sources: SourceRegistry,
): { newEvidence: number; repeated: boolean } {
  state.completedActions.push(action);
  let newEvidence = 0;
  for (const claim of claims) {
    const hash = `${claim.claim}::${[...claim.supportingSources].sort().join(',')}`;
    if (!state.evidenceHashes.includes(hash)) {
      state.evidenceHashes.push(hash);
      newEvidence++;
      state.findings.push({
        requirementId: matchRequirement(state, claim.claim),
        claim: claim.claim,
        supportingPassages: [claim.claim],
        sources: claim.supportingSources.filter((id) => sources[id]),
        confidence: claim.confidence,
      });
    }
  }
  const repeated = state.completedActions.filter((a) => a === action).length > 1;
  if (newEvidence === 0) state.roundsWithoutNewEvidence++;
  else state.roundsWithoutNewEvidence = 0;
  state.pendingActions = state.pendingActions.filter((a) => a !== action);
  return { newEvidence, repeated };
}

function matchRequirement(state: ResearchState, claim: string): string {
  const lower = claim.toLowerCase();
  const hit = state.requirements.find((r) => {
    const words = r.text.toLowerCase().split(/\s+/).filter((w) => w.length > 3);
    return words.some((w) => lower.includes(w));
  });
  return hit ? hit.id : state.requirements[0]?.id || 'REQ-1';
}

export function markContradiction(state: ResearchState, a: string, b: string, sources: string[]): void {
  state.contradictions.push({ claims: [a, b], sources });
}

export function rejectCandidate(state: ResearchState, candidate: string, reason: string): void {
  if (!state.rejected.some((r) => r.candidate === candidate)) {
    state.rejected.push({ candidate, reason });
  }
}

/** Targeted follow-up queries from missing evidence (gap-driven). */
export function followUpQueries(state: ResearchState, maxQueries = 2): Array<{ query: string; purpose: string }> {
  const out: Array<{ query: string; purpose: string }> = [];
  for (const req of state.requirements) {
    if (req.status !== 'unresolved') continue;
    const covered = state.findings.some((f) => f.requirementId === req.id);
    if (!covered && out.length < maxQueries) {
      out.push({ query: `${req.text} official details`, purpose: `Fill missing evidence for ${req.id}` });
    }
  }
  for (const question of state.openQuestions) {
    if (out.length >= maxQueries) break;
    if (!out.some((q) => q.query === question)) {
      out.push({ query: question, purpose: 'Open research question' });
    }
  }
  return out;
}

/** Before completion, classify every requirement explicitly. */
export function classifyRequirements(
  state: ResearchState,
  evidence: EvidenceClaim[],
): ResearchRequirement[] {
  return state.requirements.map((req) => {
    const supporting = evidence.filter((c) => matchRequirement({ ...state, requirements: [req] }, c.claim) === req.id);
    if (supporting.length > 0) return { ...req, status: 'answered' as const };
    if (state.roundsWithoutNewEvidence >= 2) {
      return { ...req, status: 'unresolved' as const, reason: 'No new evidence after repeated investigation' };
    }
    return { ...req, status: 'blocked' as const, reason: 'Evidence not found within the query budget' };
  });
}

export function shouldContinue(state: ResearchState, maxStagnantRounds = 2): boolean {
  if (state.roundsWithoutNewEvidence >= maxStagnantRounds) return false;
  return state.pendingActions.length > 0 || state.openQuestions.length > 0;
}
