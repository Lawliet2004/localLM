import { WebSearchEngine, formatDiagnosticReport } from '../src/web/index.ts';
import { LocalOpenAICompatibleProvider } from '../src/web/models/local_provider.ts';
import { SqliteStorage } from './web-storage.mjs';
import { PinnedPageFetcher } from './web-transport.mjs';
import { renderPage } from './web-render.mjs';
import { DocumentStore } from '../src/web/documents/document_store.ts';
import { extractAtomicClaims } from '../src/web/verification/claim_extractor.ts';
import { ClaimVerifier } from '../src/web/verification/claim_verifier.ts';
import { detectConservativeConflicts } from '../src/web/verification/evidence_assessment.ts';
import { shapeFailedFetch } from '../src/web/fetch/failure.ts';
import { ResultExtractor } from '../src/web/extraction/result_extractor.ts';
import { WeatherProvider } from '../src/web/verticals/weather/weather_provider.ts';
import { planAgentSearch } from '../src/web/planning/agent_search.ts';
import { detectFreshnessRequirement } from '../src/web/router/freshness.ts';
import { filterSnippetResults } from '../src/web/staged_research.ts';
import { sanitizeWebEvidence } from '../src/web/security/injection_guard.ts';

/** Bounded page shape returned to the model for visit/fetch-url calls. */
function shapeDocument(doc) {
  return {
    id: doc.id, url: doc.url, title: doc.title,
    extractionMethod: doc.metadata?.extractionMethod || 'main_content',
    snippetOnly: (doc.metadata?.extractionMethod || '').includes('snippet'),
    ocrRequired: Boolean(doc.metadata?.ocrRequired),
    text: (doc.text || '').slice(0, 8000),
    headings: (doc.headings || []).slice(0, 30),
    links: (doc.links || []).slice(0, 20),
  };
}

function loadDocumentStore(session) {
  const store = new DocumentStore();
  for (const doc of session.documents || []) {
    store.save({
      ...doc,
      fullText: doc.text || '',
      pages: doc.pages,
      extractionMethod: doc.metadata?.extractionMethod || 'main_content',
    });
  }
  return store;
}

let input = '';
process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) {
  input += chunk;
  if (Buffer.byteLength(input, 'utf8') > 64000) throw new Error('Research input too large');
}
let storage;
try {
  const request = JSON.parse(input);
  if (request.localModel && request.deadlineMs) request.localModel.deadlineMs = request.deadlineMs;
  storage = new SqliteStorage(request.databasePath || ':memory:');
  const llmProvider = request.localModel ? new LocalOpenAICompatibleProvider(request.localModel) : undefined;
  const engine = new WebSearchEngine({
    config: request.config,
    storage,
    fetcher: new PinnedPageFetcher(request.config?.fetch?.globalConcurrency, request.config?.fetch?.perDomainConcurrency),
    jsRender: request.config?.fetch?.jsRenderFallback
      ? (url) => renderPage(url, {
          timeoutMs: request.config.fetch.jsRenderTimeoutMs || 20000,
          maxBytes: request.config.fetch.maxBytes,
        })
      : undefined,
    llmProvider,
  });
  let result;
  if (request.action === 'health') {
    result = await engine.getHealthStatus();
    result.database = 'sqlite_ready';
  } else if (request.action === 'weather') {
    const weather = new WeatherProvider();
    const report = await weather.execute(String(request.question || ''));
    result = { answer: `${weather.formatReportAsEvidence(report)}\nSource: ${report.sourceUrl}`, sourceUrl: report.sourceUrl };
  } else if (request.action === 'open' || request.action === 'find') {
    const session = storage.load(request.sessionId);
    const doc = session.documents.find(d => d.id === request.documentId);
    if (!doc) throw new Error('Unknown document in this session');
    const store = loadDocumentStore(session);
    if (request.action === 'open') {
      // Full stored document text, not only previously compressed evidence.
      const passages = store.open(request.documentId, {
        page: request.page, section: request.section,
        passage: request.passage, offset: request.offset,
      });
      result = {
        id: doc.id, url: doc.url, title: doc.title,
        extractionMethod: doc.metadata?.extractionMethod || 'main_content',
        snippetOnly: (doc.metadata?.extractionMethod || '').includes('snippet'),
        passages,
        links: (doc.links || []).slice(0, 20),
        headings: (doc.headings || []).slice(0, 30),
      };
    } else {
      const passages = store.find(request.documentId, String(request.term || ''), 400);
      // Expandable context: each match carries its stable ref for follow-ups.
      result = {
        id: doc.id, url: doc.url, title: doc.title,
        extractionMethod: doc.metadata?.extractionMethod || 'main_content',
        snippetOnly: (doc.metadata?.extractionMethod || '').includes('snippet'),
        passages,
      };
    }
  } else if (request.action === 'search') {
    let query = request.query;
    if (typeof query === 'string' && query.trim().startsWith('[')) {
      try { query = JSON.parse(query); } catch { throw new Error('Invalid JSON-text search queries'); }
    }
    const planned = await planAgentSearch({
      queries: Array.isArray(query) ? query.map(String) : [String(query || '')],
      context: request.context, model: llmProvider,
    });
    if (planned.clarificationQuestion) {
      result = { results: [], rawCount: 0, failureCount: 0, failures: [],
        queriesUsed: [], needsClarification: true, answer: planned.clarificationQuestion };
    } else {
      const batch = await engine.searchQueries(planned.queries, detectFreshnessRequirement(planned.queries.join(' ')));
      const short = filterSnippetResults(batch.results, planned.queries.join(' '));
      const byUrl = new Map(batch.results.flatMap(hit => {
        try { return [[new URL(hit.url).href, hit]]; } catch { return []; }
      }));
      result = { ...batch, queriesUsed: planned.queries,
        results: short.map(hit => ({ ...byUrl.get(hit.url), title: hit.title, snippet: hit.snippet, url: hit.url })) };
    }
  } else if (request.action === 'fetch-url') {
    try {
      result = shapeDocument(await engine.fetchUrl(request.url));
    } catch (error) {
      result = shapeFailedFetch(String(request.url), error);
    }
  } else if (request.action === 'fetch-urls') {
    // One worker process serves the whole visit batch; per-page failures are
    // isolated so a single bad URL never discards the rest. With a goal, each
    // page is compressed by a separate extraction call into a ~150-token
    // SOURCE brief — raw page text never leaves the worker, so it cannot
    // enter the planner's context.
    const urls = (Array.isArray(request.urls) ? request.urls : []).slice(0, 4);
    const goal = String(request.goal || '').slice(0, 4000);
    const extractor = goal ? new ResultExtractor(llmProvider) : undefined;
    result = {
      pages: await Promise.all(urls.map(async (url) => {
        try {
          const doc = await engine.fetchUrl(String(url));
          if (!extractor) return shapeDocument(doc);
          return {
            url: doc.url,
            title: doc.title,
            extractionMethod: doc.metadata?.extractionMethod || 'main_content',
            snippetOnly: (doc.metadata?.extractionMethod || '').includes('snippet'),
            ocrRequired: Boolean(doc.metadata?.ocrRequired),
            text: await extractor.extractBrief(goal, doc),
            links: (doc.links || []).slice(0, 8),
          };
        } catch (error) {
          return shapeFailedFetch(String(url), error);
        }
      })),
    };
  } else if (request.action === 'verify') {
    // Bounded evidence-gap audit. Compare directly contradictory source
    // assertions before checking excerpts; never ask the model to judge itself.
    const claims = extractAtomicClaims(String(request.answer || '').slice(0, 16000));
    const evidence = (Array.isArray(request.evidence) ? request.evidence : [])
      .slice(0, 30)
      .flatMap((item, index) => String(item?.claim ?? item?.text ?? '').slice(0, 8000)
        .split(/\n+|(?<=[.!?])\s+(?=[A-Z])/).map((claim, sentence) => ({
          id: `E${index + 1}.${sentence + 1}`,
          claim,
          supportingSources: [String(item?.url || `E${index + 1}`)],
          status: 'supported',
          confidence: 1,
        })))
      .filter((item) => item.claim.trim().length > 0);
    result = new ClaimVerifier().verifyClaimsDeterministic(claims, detectConservativeConflicts(evidence));
    result.scope = 'answer';
  } else {
    const session = await (request.localModel
      ? engine.researchStaged(request.question, { mode: request.mode, context: request.context, objective: request.objective, fetchPages: request.fetchPages !== false, deadlineMs: request.deadlineMs })
      : engine.research(request.question, { mode: request.mode }));
    if (!session.clarificationQuestion && session.route?.requiresExternalData && session.trace?.queriesGenerated === 0 && session.documents.length === 0 && Object.keys(session.sources).length === 0) {
      const health = await engine.getHealthStatus();
      session.trace.searchProviderHealth = health.search;
    }
    storage.save(session);
    result = request.full ? session : {
      id: session.id, question: session.question, answer: session.answer,
      clarificationQuestion: session.clarificationQuestion,
      route: session.route, queries: session.queries.map(q => ({ query: q.query })), sources: session.sources, evidence: session.evidence,
      verification: session.verification, tokenUsage: session.tokenUsage, limitations: session.limitations,
      trace: session.trace, startedAt: session.startedAt, completedAt: session.completedAt,
      documents: session.documents.map(d => ({ id: d.id, title: d.title, url: d.url })),
    };
    if (request.trace) process.stderr.write(formatDiagnosticReport(session) + '\n');
  }
  await new Promise((resolveWrite, rejectWrite) => process.stdout.write(JSON.stringify(result,
    (_key, value) => typeof value === 'string' ? sanitizeWebEvidence(value).sanitizedText : value), error => error ? rejectWrite(error) : resolveWrite()));
  // A deadline race can leave transports pending. The bounded partial result
  // has drained to stdout; close SQLite before ending the worker and its job.
  if (result.limitations?.some(item => /deadline/i.test(item))) { storage.close(); storage = undefined; process.exit(0); }
} catch (error) {
  process.stderr.write(String(error.message || error) + '\n');
  process.exitCode = 1;
} finally { storage?.close(); }
