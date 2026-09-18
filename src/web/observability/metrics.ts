/**
 * Metrics & Diagnostics: Formats and evaluates research efficiency and grounding performance.
 */

import type { ResearchSession } from '../types';

export function formatDiagnosticReport(session: ResearchSession): string {
  const t = session.trace;
  const compressionRatio =
    t.finalEvidenceTokens > 0
      ? (t.extractedTokens / t.finalEvidenceTokens).toFixed(1) + 'x'
      : 'N/A';

  const verifiedFrac =
    t.evidenceClaims > 0 ? `${t.verifiedClaims}/${t.evidenceClaims}` : '0/0';

  const citationsCount = session.answer
    ? (session.answer.match(/\[S\d+\]/g) || []).length
    : 0;

  const queryFailures = (t.queryFailures || []).map((f) => `  - ${f.query}: ${f.error}`).join('\n');
  const diagnostics = t.searxngDiagnostics ? `\nEngine diagnostics: ${JSON.stringify(t.searxngDiagnostics)}` : '';
  const pagination = t.paginationPages?.length ? `\nPagination pages fetched: ${t.paginationPages.join(', ')}` : '';

  return `
--- RESEARCH DIAGNOSTICS REPORT ---
Question: ${session.question}
Routing: ${session.route.vertical} (${session.route.complexity || 'medium'}, Freshness: ${session.route.freshness})
Provider: ${t.providerUsed}${t.fallbackUsed ? ` (fallback: ${t.fallbackReason})` : t.fallbackReason ? ` (${t.fallbackReason})` : ''}
Queries: ${t.queriesGenerated}${t.searchFailureCount ? ` (${t.searchFailureCount} failed)` : ''}
${session.queries.map((query, index) => `  ${index + 1}. ${query.query}`).join('\n')}${queryFailures ? `\nQuery failures:\n${queryFailures}` : ''}${diagnostics}${pagination}
Raw results: ${t.searchResults}
Unique results: ${t.uniqueResults}
Pages selected: ${t.pagesSelected}
Pages successfully fetched: ${t.pagesFetched} (Failures: ${t.fetchFailures})
Extracted tokens: ${t.extractedTokens.toLocaleString()}
Chunks: ${t.chunksCreated}
Hybrid candidates: ${t.chunksAfterRetrieval}
Reranked chunks: ${t.chunksAfterRerank}
Evidence statements: ${t.evidenceClaims}
Evidence tokens passed to final LLM: ${t.finalEvidenceTokens.toLocaleString()}
Total final prompt tokens: ${t.finalPromptTokens.toLocaleString()}
Information Compression Ratio: ${compressionRatio} (examined ~${t.extractedTokens} tokens -> sent ~${t.finalEvidenceTokens} tokens)
Citations rendered: ${citationsCount}
Verified factual claims: ${verifiedFrac}
Unsupported claims: ${t.unsupportedClaims}
Conflicting claims: ${t.conflictingClaims}
Total latency: ${t.totalLatencyMs}ms
-----------------------------------`.trim();
}
