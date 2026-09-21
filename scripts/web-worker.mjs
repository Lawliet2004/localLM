import { WebSearchEngine, formatDiagnosticReport } from '../src/web/index.ts';
import { LocalOpenAICompatibleProvider } from '../src/web/models/local_provider.ts';
import { SqliteStorage } from './web-storage.mjs';
import { PinnedPageFetcher } from './web-transport.mjs';
import { renderPage } from './web-render.mjs';
import { DocumentStore } from '../src/web/documents/document_store.ts';
import { extractAtomicClaims } from '../src/web/verification/claim_extractor.ts';
import { ClaimVerifier } from '../src/web/verification/claim_verifier.ts';

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

function loadDocumentStore(storage, sessionId) {
  const store = new DocumentStore();
  try {
    const session = storage.load(sessionId);
    for (const doc of session.documents || []) {
      store.save({
        ...doc,
        fullText: doc.text || '',
        pages: doc.pages,
        extractionMethod: doc.metadata?.extractionMethod || 'main_content',
      });
    }
  } catch { /* unknown session surfaces below */ }
  return store;
}

let input = '';
for await (const chunk of process.stdin) {
  input += chunk;
  if (input.length > 64000) throw new Error('Research input too large');
}
let storage;
try {
  const request = JSON.parse(input);
  storage = new SqliteStorage(request.databasePath || ':memory:');
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
    llmProvider: request.localModel ? new LocalOpenAICompatibleProvider(request.localModel) : undefined,
  });
  let result;
  if (request.action === 'health') {
    result = await engine.getHealthStatus();
    result.database = 'sqlite_ready';
  } else if (request.action === 'open' || request.action === 'find') {
    const session = storage.load(request.sessionId);
    const doc = session.documents.find(d => d.id === request.documentId);
    if (!doc) throw new Error('Unknown document in this session');
    const store = loadDocumentStore(storage, request.sessionId);
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
    result = await engine.searchQueries(request.query);
  } else if (request.action === 'fetch-url') {
    result = shapeDocument(await engine.fetchUrl(request.url));
  } else if (request.action === 'fetch-urls') {
    // One worker process serves the whole visit batch; per-page failures are
    // isolated so a single bad URL never discards the rest.
    const urls = (Array.isArray(request.urls) ? request.urls : []).slice(0, 4);
    result = {
      pages: await Promise.all(urls.map(async (url) => {
        try {
          return shapeDocument(await engine.fetchUrl(String(url)));
        } catch (error) {
          return { url: String(url), isError: true, message: String(error?.message || error) };
        }
      })),
    };
  } else if (request.action === 'verify') {
    // Post-finish audit: deterministic lexical check of the answer's atomic
    // claims against collected evidence text. Never calls the model.
    const claims = extractAtomicClaims(String(request.answer || '').slice(0, 16000));
    const evidence = (Array.isArray(request.evidence) ? request.evidence : [])
      .slice(0, 30)
      .map((item, index) => ({
        id: `E${index + 1}`,
        claim: String(item?.claim ?? item?.text ?? '').slice(0, 8000),
        supportingSources: [String(item?.url || `E${index + 1}`)],
        status: 'supported',
        confidence: 1,
      }))
      .filter((item) => item.claim.trim().length > 0);
    result = new ClaimVerifier().verifyClaimsDeterministic(claims, evidence);
  } else {
    const session = await engine.research(request.question, { mode: request.mode });
    storage.save(session);
    result = request.full ? session : {
      id: session.id, question: session.question, answer: session.answer,
      route: session.route, sources: session.sources, evidence: session.evidence,
      verification: session.verification, tokenUsage: session.tokenUsage,
      trace: session.trace, startedAt: session.startedAt, completedAt: session.completedAt,
      documents: session.documents.map(d => ({ id: d.id, title: d.title, url: d.url })),
    };
    if (request.trace) process.stderr.write(formatDiagnosticReport(session) + '\n');
  }
  process.stdout.write(JSON.stringify(result));
} catch (error) {
  process.stderr.write(String(error.message || error) + '\n');
  process.exitCode = 1;
} finally { storage?.close(); }
