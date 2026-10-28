import { createRequire } from "module";
import { DatabaseSync } from "node:sqlite";
import { lookup } from "node:dns/promises";
import { BlockList, isIP } from "node:net";
import http from "node:http";
import https from "node:https";
import { createBrotliDecompress, createGunzip, createInflate } from "node:zlib";
import { existsSync } from "node:fs";
import { join } from "node:path";
//#region src/web/cache/sqlite.ts
var InMemoryStorageAdapter = class {
	stores = /* @__PURE__ */ new Map();
	getTable(table) {
		let t = this.stores.get(table);
		if (!t) {
			t = /* @__PURE__ */ new Map();
			this.stores.set(table, t);
		}
		return t;
	}
	async get(table, key) {
		const t = this.getTable(table);
		const entry = t.get(key);
		if (!entry) return null;
		if (Date.now() > entry.expiresAt) {
			t.delete(key);
			return null;
		}
		return entry.value;
	}
	async set(table, key, value, ttlSeconds = 86400) {
		this.getTable(table).set(key, {
			value,
			expiresAt: Date.now() + ttlSeconds * 1e3
		});
	}
	async delete(table, key) {
		this.getTable(table).delete(key);
	}
	async clear(table) {
		if (table) this.stores.get(table)?.clear();
		else this.stores.clear();
	}
};
//#endregion
//#region src/web/cache/search_cache.ts
var FRESHNESS_TTL_SECONDS = {
	realtime: 300,
	day: 3600,
	week: 21600,
	month: 86400,
	year: 604800,
	any: 86400
};
function ttlForFreshness(freshness, fallback) {
	return Math.min(fallback, FRESHNESS_TTL_SECONDS[freshness] ?? fallback);
}
//#endregion
//#region src/web/ranking/url_normalizer.ts
/**
* URL Normalizer: Normalizes URLs for canonical deduplication while preserving
* meaningful identifying parameters and removing tracking/marketing parameters.
*/
var TRACKING_PARAMS = /* @__PURE__ */ new Set([
	"utm_source",
	"utm_medium",
	"utm_campaign",
	"utm_term",
	"utm_content",
	"utm_id",
	"fbclid",
	"gclid",
	"gclsrc",
	"dclid",
	"msclkid",
	"mc_eid",
	"mc_cid",
	"_hsenc",
	"_hsmi",
	"yclid",
	"twclid",
	"igshid"
]);
function normalizeUrl(rawUrl) {
	try {
		const url = new URL(rawUrl.trim());
		url.protocol = url.protocol.toLowerCase();
		url.hostname = url.hostname.toLowerCase();
		if (url.protocol === "http:" && url.port === "80" || url.protocol === "https:" && url.port === "443") url.port = "";
		if (url.hostname.startsWith("www.")) url.hostname = url.hostname.substring(4);
		url.hash = "";
		const searchParams = new URLSearchParams(url.search);
		const keysToDelete = [];
		for (const key of searchParams.keys()) {
			const lowerKey = key.toLowerCase();
			if (TRACKING_PARAMS.has(lowerKey) || lowerKey.startsWith("utm_")) keysToDelete.push(key);
		}
		for (const key of keysToDelete) searchParams.delete(key);
		const sortedKeys = Array.from(new Set(searchParams.keys())).sort();
		const sortedParams = new URLSearchParams();
		for (const k of sortedKeys) {
			const values = searchParams.getAll(k);
			for (const v of values) sortedParams.append(k, v);
		}
		const queryStr = sortedParams.toString();
		url.search = queryStr ? `?${queryStr}` : "";
		let path = url.pathname;
		if (path.length > 1 && path.endsWith("/")) path = path.slice(0, -1);
		url.pathname = path;
		return url.toString();
	} catch {
		return rawUrl.trim();
	}
}
/**
* Extracts a normalized domain hostname.
*/
function extractDomain(urlStr) {
	try {
		let host = new URL(urlStr).hostname.toLowerCase();
		if (host.startsWith("www.")) host = host.substring(4);
		return host;
	} catch {
		return "unknown";
	}
}
//#endregion
//#region src/web/cache/document_cache.ts
var DocumentCache = class {
	storage;
	constructor(storage = new InMemoryStorageAdapter()) {
		this.storage = storage;
	}
	async get(url) {
		const norm = normalizeUrl(url);
		return this.storage.get("document_cache", norm);
	}
	async set(doc, ttlSeconds = 259200) {
		const norm = normalizeUrl(doc.url);
		await this.storage.set("document_cache", norm, doc, ttlSeconds);
	}
};
//#endregion
//#region src/web/types.ts
function emptySearchMeta() {
	return {
		answers: [],
		infoboxes: [],
		corrections: [],
		suggestions: []
	};
}
/** Merge SearXNG meta from concurrent/paginated queries; first-seen wins. */
function mergeSearchMeta(base, extra) {
	const answers = [...base.answers];
	for (const a of extra.answers) if (!answers.some((x) => x.answer === a.answer)) answers.push(a);
	const infoboxes = [...base.infoboxes];
	for (const ib of extra.infoboxes) if (!infoboxes.some((x) => x.title === ib.title && x.content === ib.content)) infoboxes.push(ib);
	const uniq = (items) => [...new Set(items.map((s) => s.trim()).filter(Boolean))];
	return {
		answers: answers.slice(0, 5),
		infoboxes: infoboxes.slice(0, 3),
		corrections: uniq([...base.corrections, ...extra.corrections]).slice(0, 3),
		suggestions: uniq([...base.suggestions, ...extra.suggestions]).slice(0, 8)
	};
}
//#endregion
//#region src/web/config/defaults.ts
var DEFAULT_CONFIG = {
	enabled: true,
	mode: "normal",
	profile: "standard",
	searxngBaseUrl: "http://127.0.0.1:8080",
	searxngTimeoutMs: 15e3,
	searxngEngines: [],
	searxngDisabledEngines: [],
	searchProvider: "searxng",
	googleApiKey: "",
	googleCxId: "",
	searchFallback: {
		enabled: true,
		googleDailyLimit: 90
	},
	maxConcurrentQueries: 4,
	searchRetries: 1,
	searchRetryDelayMs: 1e3,
	fetch: {
		timeoutSeconds: 10,
		maxBytes: 5242880,
		globalConcurrency: 8,
		perDomainConcurrency: 2,
		userAgent: "LocalLM-Research/1.0 (+https://github.com/locallm/desktop)",
		waybackFallback: true,
		domainLearning: true,
		jsRenderFallback: false,
		jsRenderTimeoutMs: 2e4
	},
	cache: {
		enabled: true,
		databasePath: "./data/web-cache.sqlite",
		searchTtlSeconds: {
			news: 3600,
			weather: 1800,
			documentation: 604800,
			default: 86400
		}
	}
};
/**
* Environment overrides, applied last so a deployment can pin the SearXNG
* endpoint and engine catalogue without touching app settings (the app always
* injects a base URL, so env has to win to be useful):
* SEARXNG_BASE_URL, SEARXNG_TIMEOUT_MS, SEARXNG_ENGINES (csv),
* SEARXNG_DISABLED_ENGINES (csv), AGC_WEB_PROFILE (=low_memory).
*/
function envOverrides() {
	const env = globalThis.process?.env;
	if (!env) return {};
	const out = {};
	const list = (value) => value?.split(",").map((s) => s.trim()).filter(Boolean) ?? [];
	if (env.SEARXNG_BASE_URL?.trim()) out.searxngBaseUrl = env.SEARXNG_BASE_URL.trim();
	const timeout = Number(env.SEARXNG_TIMEOUT_MS);
	if (Number.isFinite(timeout) && timeout >= 1e3 && timeout <= 12e4) out.searxngTimeoutMs = Math.floor(timeout);
	const engines = list(env.SEARXNG_ENGINES);
	if (engines.length) out.searxngEngines = engines;
	const disabled = list(env.SEARXNG_DISABLED_ENGINES);
	if (disabled.length) out.searxngDisabledEngines = disabled;
	if (env.AGC_WEB_PROFILE === "low_memory") out.profile = "low_memory";
	return out;
}
function createConfig(overrides) {
	const env = envOverrides();
	const base = {
		...DEFAULT_CONFIG,
		...overrides,
		...env
	};
	for (const key of [
		"fetch",
		"cache",
		"searchFallback"
	]) Object.assign(base, { [key]: {
		...DEFAULT_CONFIG[key],
		...overrides?.[key]
	} });
	if (base.profile === "low_memory") return {
		...base,
		maxConcurrentQueries: Math.min(base.maxConcurrentQueries, 2),
		fetch: {
			...base.fetch,
			globalConcurrency: 4,
			perDomainConcurrency: 1
		}
	};
	return base;
}
//#endregion
//#region src/web/ranking/deduplicator.ts
/**
* Computes Token Jaccard similarity between two strings.
*/
function tokenJaccardSimilarity(a, b) {
	const tokenize = (s) => new Set(s.toLowerCase().replace(/[^\w\s]/g, "").split(/\s+/).filter((t) => t.length > 1));
	const setA = tokenize(a);
	const setB = tokenize(b);
	if (setA.size === 0 && setB.size === 0) return 1;
	if (setA.size === 0 || setB.size === 0) return 0;
	let intersectionSize = 0;
	for (const token of setA) if (setB.has(token)) intersectionSize++;
	const unionSize = setA.size + setB.size - intersectionSize;
	return unionSize === 0 ? 0 : intersectionSize / unionSize;
}
/**
* Computes normalized Levenshtein similarity [0.0 - 1.0].
*/
function levenshteinSimilarity(a, b) {
	const s1 = a.toLowerCase().trim();
	const s2 = b.toLowerCase().trim();
	if (s1 === s2) return 1;
	if (s1.length === 0 || s2.length === 0) return 0;
	const m = s1.length;
	const n = s2.length;
	const dp = Array.from({ length: m + 1 }, () => new Array(n + 1).fill(0));
	for (let i = 0; i <= m; i++) dp[i][0] = i;
	for (let j = 0; j <= n; j++) dp[0][j] = j;
	for (let i = 1; i <= m; i++) for (let j = 1; j <= n; j++) {
		const cost = s1[i - 1] === s2[j - 1] ? 0 : 1;
		dp[i][j] = Math.min(dp[i - 1][j] + 1, dp[i][j - 1] + 1, dp[i - 1][j - 1] + cost);
	}
	return 1 - dp[m][n] / Math.max(m, n);
}
function deduplicateSearchResults(results) {
	const seenUrls = /* @__PURE__ */ new Map();
	const uniqueResults = [];
	for (const result of results) {
		const normUrl = normalizeUrl(result.url);
		const normCanonical = result.canonicalUrl ? normalizeUrl(result.canonicalUrl) : void 0;
		const existing = seenUrls.get(normUrl) || (normCanonical ? seenUrls.get(normCanonical) : void 0);
		if (existing) {
			if (result.queryId && existing.queryId && !existing.queryId.includes(result.queryId)) existing.queryId = `${existing.queryId},${result.queryId}`;
			continue;
		}
		let isDuplicate = false;
		for (const u of uniqueResults) {
			const jaccard = tokenJaccardSimilarity(u.title, result.title);
			if (u.domain === result.domain && jaccard > .75) {
				isDuplicate = true;
				if (result.queryId && u.queryId && !u.queryId.includes(result.queryId)) u.queryId = `${u.queryId},${result.queryId}`;
				break;
			}
			if (u.domain !== result.domain && (jaccard >= .88 || levenshteinSimilarity(u.title, result.title) >= .9)) {
				isDuplicate = true;
				if (result.queryId && u.queryId && !u.queryId.includes(result.queryId)) u.queryId = `${u.queryId},${result.queryId}`;
				break;
			}
		}
		if (!isDuplicate) {
			seenUrls.set(normUrl, result);
			if (normCanonical) seenUrls.set(normCanonical, result);
			uniqueResults.push(result);
		}
	}
	return uniqueResults;
}
//#endregion
//#region src/web/search/result_fusion.ts
function fuseSearchResults(queryResults) {
	const allResults = [];
	for (const group of queryResults) for (const r of group.results) allResults.push({
		...r,
		queryId: r.queryId || group.query
	});
	return deduplicateSearchResults(allResults);
}
//#endregion
//#region src/web/search/result_normalizer.ts
function normalizeRawSearchResult(raw, queryId, rank, page = 1) {
	const url = String(raw.url || raw.link || "").trim();
	const title = String(raw.title || "Untitled").trim();
	const snippet = String(raw.content || raw.snippet || raw.body || "").trim();
	const domain = extractDomain(url);
	const publishedAt = raw.publishedDate || raw.publishedAt || raw.date;
	return {
		id: page > 1 ? `${queryId}-P${page}-R${rank}` : `${queryId}-R${rank}`,
		queryId,
		title,
		url,
		snippet,
		domain,
		publishedAt: publishedAt ? String(publishedAt) : void 0,
		engine: raw.engine ? String(raw.engine) : void 0,
		rank: (page - 1) * 10 + rank,
		score: typeof raw.score === "number" ? raw.score : void 0,
		metadata: {
			...raw.metadata || {},
			...page > 1 ? { searxngPage: page } : {}
		}
	};
}
//#endregion
//#region src/web/extraction/metadata.ts
function extractPageMetadata(html) {
	const meta = {};
	const canonicalMatch = html.match(/<link\s+[^>]*rel=["']canonical["'][^>]*href=["']([^"']+)["']/i) || html.match(/<link\s+[^>]*href=["']([^"']+)["'][^>]*rel=["']canonical["']/i);
	if (canonicalMatch) meta.canonicalUrl = canonicalMatch[1].trim();
	const ogTitleMatch = html.match(/<meta\s+[^>]*property=["']og:title["'][^>]*content=["']([^"']+)["']/i) || html.match(/<meta\s+[^>]*content=["']([^"']+)["'][^>]*property=["']og:title["']/i);
	if (ogTitleMatch) meta.title = ogTitleMatch[1].trim();
	else {
		const titleTagMatch = html.match(/<title[^>]*>([\s\S]*?)<\/title>/i);
		if (titleTagMatch) meta.title = titleTagMatch[1].replace(/\s+/g, " ").trim();
		else {
			const h1Match = html.match(/<h1[^>]*>([\s\S]*?)<\/h1>/i);
			if (h1Match) meta.title = h1Match[1].replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim();
		}
	}
	const descMatch = html.match(/<meta\s+[^>]*name=["']description["'][^>]*content=["']([^"']+)["']/i) || html.match(/<meta\s+[^>]*property=["']og:description["'][^>]*content=["']([^"']+)["']/i);
	if (descMatch) meta.description = descMatch[1].trim();
	const authorMatch = html.match(/<meta\s+[^>]*name=["']author["'][^>]*content=["']([^"']+)["']/i) || html.match(/<meta\s+[^>]*property=["']article:author["'][^>]*content=["']([^"']+)["']/i);
	if (authorMatch) meta.author = authorMatch[1].trim();
	const dateMatch = html.match(/<meta\s+[^>]*property=["']article:published_time["'][^>]*content=["']([^"']+)["']/i) || html.match(/<meta\s+[^>]*name=["']publish-date["'][^>]*content=["']([^"']+)["']/i) || html.match(/<meta\s+[^>]*name=["']date["'][^>]*content=["']([^"']+)["']/i) || html.match(/<meta\s+[^>]*itemprop=["']datePublished["'][^>]*content=["']([^"']+)["']/i);
	if (dateMatch) meta.publishedAt = dateMatch[1].trim();
	const jsonLdMatches = html.matchAll(/<script\s+[^>]*type=["']application\/ld\+json["'][^>]*>([\s\S]*?)<\/script>/gi);
	for (const match of jsonLdMatches) try {
		const parsed = JSON.parse(match[1]);
		const items = Array.isArray(parsed) ? parsed : [parsed];
		for (const item of items) {
			if (!meta.publishedAt && item.datePublished) meta.publishedAt = String(item.datePublished);
			if (!meta.author && item.author) {
				if (typeof item.author === "string") meta.author = item.author;
				else if (item.author.name) meta.author = item.author.name;
			}
			if (!meta.title && item.headline) meta.title = String(item.headline);
			if (!meta.articleBody && typeof item.articleBody === "string" && item.articleBody.trim().length > 200) meta.articleBody = item.articleBody.trim();
		}
	} catch {}
	return meta;
}
//#endregion
//#region src/web/extraction/main_content.ts
/**
* Main Content Extractor: Extracts readable article/page text, strips boilerplate
* (nav, ads, cookie notices, scripts, footers), preserves heading structure,
* and generates content fingerprints.
*/
/**
* Computes a fast deterministic SHA-256 or djb2 hash string.
*/
function computeContentHash(text) {
	let h1 = 3735928559;
	let h2 = 1103547991;
	for (let i = 0; i < text.length; i++) {
		const ch = text.charCodeAt(i);
		h1 = Math.imul(h1 ^ ch, 2654435761);
		h2 = Math.imul(h2 ^ ch, 1597334677);
	}
	h1 = Math.imul(h1 ^ h1 >>> 16, 2246822507) ^ Math.imul(h2 ^ h2 >>> 13, 3266489909);
	h2 = Math.imul(h2 ^ h2 >>> 16, 2246822507) ^ Math.imul(h1 ^ h1 >>> 13, 3266489909);
	return (4294967296 * (2097151 & h2) + (h1 >>> 0)).toString(16);
}
function extractMainContent(html, fallbackTitle = "") {
	const meta = extractPageMetadata(html);
	if (!/<[a-z][\s\S]*>/i.test(html)) {
		const clean = html.trim();
		return {
			title: meta.title || fallbackTitle,
			author: meta.author,
			publishedAt: meta.publishedAt,
			canonicalUrl: meta.canonicalUrl,
			description: meta.description,
			text: clean,
			contentHash: computeContentHash(clean),
			characters: clean.length,
			confidence: .9,
			method: "plain_text",
			headings: [],
			links: [],
			tables: []
		};
	}
	let processed = html.replace(/<script\b[^<]*(?:(?!<\/script>)<[^<]*)*<\/script>/gi, "").replace(/<style\b[^<]*(?:(?!<\/style>)<[^<]*)*<\/style>/gi, "").replace(/<noscript\b[^<]*(?:(?!<\/noscript>)<[^<]*)*<\/noscript>/gi, "").replace(/<svg\b[^<]*(?:(?!<\/svg>)<[^<]*)*<\/svg>/gi, "").replace(/<!--[\s\S]*?-->/g, "");
	for (const tag of [
		"nav",
		"header",
		"footer",
		"aside",
		"form",
		"dialog"
	]) {
		const regex = new RegExp(`<${tag}\\b[^<]*(?:(?!<\\/${tag}>)<[^<]*)*<\\/${tag}>`, "gi");
		processed = processed.replace(regex, "");
	}
	processed = processed.replace(/<div[^>]*(id|class)=["'][^"']*(cookie|banner|advertisement|sponsor|popup|modal|consent)[^"']*["'][^<]*(?:(?!<\/div>)<[^<]*)*<\/div>/gi, "");
	let articleContent = "";
	const mainOrArticleMatch = processed.match(/<article\b[^>]*>([\s\S]*?)<\/article>/i) || processed.match(/<main\b[^>]*>([\s\S]*?)<\/main>/i);
	let method = "heuristic_strip";
	if (mainOrArticleMatch && mainOrArticleMatch[1].trim().length > 300) {
		articleContent = mainOrArticleMatch[1];
		method = "article_dom";
	} else articleContent = processed;
	const headings = [];
	const headingTexts = [...articleContent.matchAll(/<h[1-6]\b[^>]*>([\s\S]*?)<\/h[1-6]>/gi)].map((m) => m[1].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim()).filter(Boolean);
	headings.push(...headingTexts.slice(0, 50));
	const links = [...articleContent.matchAll(/<a\b[^>]*href=["']([^"']+)["'][^>]*>([\s\S]*?)<\/a>/gi)].map((m) => ({
		href: m[1].trim(),
		text: m[2].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim().slice(0, 200)
	})).filter((l) => l.href && /^https?:\/\//i.test(l.href)).slice(0, 100);
	const tables = [...articleContent.matchAll(/<table\b[^>]*>([\s\S]*?)<\/table>/gi)].slice(0, 10).flatMap((m) => [...m[1].matchAll(/<tr\b[^>]*>([\s\S]*?)<\/tr>/gi)].slice(0, 30).map((row) => [...row[1].matchAll(/<t[hd]\b[^>]*>([\s\S]*?)<\/t[hd]>/gi)].map((cell) => cell[1].replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").trim().slice(0, 300)).filter(Boolean).join(" | ")).filter((row) => row.length > 0));
	articleContent = articleContent.replace(/<h1\b[^>]*>([\s\S]*?)<\/h1>/gi, "\n\n# $1\n\n");
	articleContent = articleContent.replace(/<h2\b[^>]*>([\s\S]*?)<\/h2>/gi, "\n\n## $1\n\n");
	articleContent = articleContent.replace(/<h3\b[^>]*>([\s\S]*?)<\/h3>/gi, "\n\n### $1\n\n");
	articleContent = articleContent.replace(/<h[4-6]\b[^>]*>([\s\S]*?)<\/h[4-6]>/gi, "\n\n#### $1\n\n");
	articleContent = articleContent.replace(/<p\b[^>]*>([\s\S]*?)<\/p>/gi, "\n\n$1\n\n");
	articleContent = articleContent.replace(/<br\s*\/?>/gi, "\n");
	articleContent = articleContent.replace(/<li\b[^>]*>([\s\S]*?)<\/li>/gi, "\n- $1");
	articleContent = articleContent.replace(/<th\b[^>]*>([\s\S]*?)<\/th>/gi, " | $1");
	articleContent = articleContent.replace(/<td\b[^>]*>([\s\S]*?)<\/td>/gi, " | $1");
	articleContent = articleContent.replace(/<\/tr>/gi, " |\n");
	let text = articleContent.replace(/<[^>]+>/g, " ");
	text = text.replace(/&nbsp;/gi, " ").replace(/&amp;/gi, "&").replace(/&lt;/gi, "<").replace(/&gt;/gi, ">").replace(/&quot;/gi, "\"").replace(/&#39;/gi, "'").replace(/&#x27;/gi, "'").replace(/&mdash;/gi, "—").replace(/&ndash;/gi, "–");
	let cleanText = text.split("\n").map((l) => l.trim()).filter((l) => l.length > 0).join("\n\n");
	if (meta.articleBody && meta.articleBody.length > 200 && (cleanText.length < 500 || meta.articleBody.length > cleanText.length)) {
		cleanText = meta.articleBody;
		method = "article_dom";
	}
	const confidence = cleanText.length > 500 ? method === "article_dom" ? .95 : .85 : .6;
	return {
		title: meta.title || fallbackTitle,
		author: meta.author,
		publishedAt: meta.publishedAt,
		canonicalUrl: meta.canonicalUrl,
		description: meta.description,
		text: cleanText,
		contentHash: computeContentHash(cleanText),
		characters: cleanText.length,
		confidence,
		method,
		headings,
		links,
		tables
	};
}
//#endregion
//#region src/web/search/searxng_provider.ts
/**
* Normalize SearXNG's version-varying meta fields into SearchMeta. Answers may
* be plain strings or {answer,url} objects; infoboxes expose `infobox`/`content`
* with optional `urls`. Empty entries are dropped.
*/
function normalizeSearxMeta(data) {
	const meta = emptySearchMeta();
	if (Array.isArray(data?.answers)) meta.answers = data.answers.map((a) => typeof a === "string" ? { answer: a } : {
		answer: String(a?.answer ?? a?.content ?? ""),
		url: a?.url ? String(a.url) : void 0
	}).filter((a) => a.answer.trim().length > 0).slice(0, 5);
	if (Array.isArray(data?.infoboxes)) meta.infoboxes = data.infoboxes.map((ib) => ({
		title: String(ib?.infobox ?? ib?.title ?? ""),
		content: String(ib?.content ?? ""),
		url: ib?.urls?.[0]?.url ? String(ib.urls[0].url) : void 0
	})).filter((ib) => ib.title.trim().length > 0 || ib.content.trim().length > 0).slice(0, 3);
	if (Array.isArray(data?.corrections)) meta.corrections = data.corrections.map((c) => typeof c === "string" ? c : String(c?.title ?? "")).filter((c) => c.trim().length > 0).slice(0, 3);
	if (Array.isArray(data?.suggestions)) meta.suggestions = data.suggestions.filter((s) => typeof s === "string" && s.trim().length > 0).slice(0, 8);
	return meta;
}
var resultMeta = /* @__PURE__ */ new WeakMap();
/** Meta attached to a `search()` return value so concurrent queries cannot clobber each other. */
function metaForResults(results) {
	return resultMeta.get(results);
}
function parseSearxngBaseUrls(baseUrl) {
	const urls = baseUrl.split(/[,;]/).map((s) => s.trim()).filter(Boolean);
	return [...new Set((urls.length ? urls : ["http://127.0.0.1:8080"]).map((value) => {
		let url;
		try {
			url = new URL(value);
		} catch {
			throw new Error("SearXNG base URL must be an absolute HTTP or HTTPS URL.");
		}
		if (!["http:", "https:"].includes(url.protocol) || url.username || url.password || url.search || url.hash) throw new Error("SearXNG base URL must use HTTP or HTTPS without credentials, a query, or a fragment.");
		url.pathname = url.pathname.replace(/\/+$/, "").replace(/\/search$/, "") + "/";
		return url.href.replace(/\/$/, "");
	}))];
}
function usableMetaUrl(url) {
	if (!url) return void 0;
	try {
		const parsed = new URL(url);
		if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return void 0;
		return parsed.href;
	} catch {
		return;
	}
}
var SearXNGError = class extends Error {
	diagnostics;
	retryable;
	constructor(message, diagnostics, retryable = true) {
		super(message);
		this.diagnostics = diagnostics;
		this.retryable = retryable;
		this.name = "SearXNGError";
	}
};
function engineFailures(data) {
	const failures = {};
	if (data.engines && !Array.isArray(data.engines) && typeof data.engines === "object") {
		for (const [name, info] of Object.entries(data.engines)) if (info?.error || info?.timed_out) failures[name] = String(info.error || "timeout");
	}
	for (const entry of Array.isArray(data.unresponsive_engines) ? data.unresponsive_engines : []) if (Array.isArray(entry) && typeof entry[0] === "string") failures[entry[0]] = String(entry[1] || "unresponsive");
	else if (typeof entry === "string") failures[entry] = "unresponsive";
	return failures;
}
var SearXNGProvider = class SearXNGProvider {
	baseUrl;
	timeoutMs;
	engines;
	disabledEngines;
	retryAfter = /* @__PURE__ */ new Map();
	lastDiagnostics = null;
	/** Metadata from the last completed search; per-query metadata travels with results. */
	lastMeta = null;
	constructor(baseUrl = "http://127.0.0.1:8080", timeoutMs = 15e3, engines = [], disabledEngines = []) {
		this.baseUrl = baseUrl;
		this.timeoutMs = timeoutMs;
		this.engines = engines;
		this.disabledEngines = disabledEngines;
	}
	/** Probe the real JSON search API, including upstream engine failures. */
	async checkHealth() {
		try {
			const hits = await this.search({
				query: "SearXNG",
				maxResults: 1
			});
			const degraded = Object.keys(this.lastDiagnostics?.engines || {}).length > 0;
			const hasEvidence = hits.length || this.lastMeta?.answers.length || this.lastMeta?.infoboxes.length;
			return {
				search: degraded ? "degraded" : hasEvidence ? "healthy" : "empty",
				...!hasEvidence ? { searchError: "SearXNG JSON API responded, but the probe returned no results. Check the enabled engines." } : {},
				searxngDiagnostics: this.lastDiagnostics ?? void 0
			};
		} catch (error) {
			return {
				search: this.lastDiagnostics?.httpStatus ? "error" : "unreachable",
				searchError: error instanceof Error ? error.message : String(error),
				searxngDiagnostics: this.lastDiagnostics ?? void 0
			};
		}
	}
	/** HTTP 200 alone does not establish that an engine actually answered. */
	async checkEngineHealth() {
		const working = [];
		const failing = {};
		const queue = [...this.engines];
		await Promise.all(Array.from({ length: Math.min(4, queue.length) }, async () => {
			while (queue.length) {
				const engine = queue.shift();
				try {
					const probe = new SearXNGProvider(this.baseUrl, this.timeoutMs, [engine]);
					const hits = await probe.search({
						query: "SearXNG",
						maxResults: 1
					});
					const reason = probe.lastDiagnostics?.engines?.[engine];
					if (reason) failing[engine] = reason;
					else if (hits.length || probe.lastMeta?.answers.length || probe.lastMeta?.infoboxes.length) working.push(engine);
					else failing[engine] = "No results returned for the probe";
				} catch (error) {
					failing[engine] = error instanceof Error ? error.message : String(error);
				}
			}
		}));
		return {
			working,
			failing
		};
	}
	async search(request) {
		this.lastMeta = null;
		this.lastDiagnostics = null;
		if (typeof request.query !== "string" || !request.query.trim()) throw new SearXNGError("SearXNG requires a nonempty search query.", {}, false);
		let bases;
		try {
			bases = parseSearxngBaseUrls(this.baseUrl);
		} catch (error) {
			const message = error instanceof Error ? error.message : String(error);
			this.lastDiagnostics = { error: message };
			throw new SearXNGError(message, this.lastDiagnostics, false);
		}
		const deadline = Date.now() + this.timeoutMs;
		let lastError;
		const available = bases.filter((base) => (this.retryAfter.get(base) || 0) <= Date.now());
		if (!available.length) {
			this.lastDiagnostics = { error: "SearXNG Retry-After cooldown active for all configured instances." };
			throw new SearXNGError(this.lastDiagnostics.error, this.lastDiagnostics, false);
		}
		for (let i = 0; i < available.length; i++) {
			const remaining = deadline - Date.now();
			if (remaining <= 0) break;
			try {
				const outcome = await this.searchAgainst(available[i], request, remaining / (available.length - i));
				this.lastDiagnostics = outcome.diagnostics;
				this.lastMeta = outcome.meta;
				resultMeta.set(outcome.results, outcome.meta);
				return outcome.results;
			} catch (error) {
				lastError = error;
				this.lastDiagnostics = lastError.diagnostics;
			}
		}
		throw lastError || new SearXNGError("SearXNG request timed out.", { error: "timeout" });
	}
	async searchAgainst(baseUrl, request, timeoutMs) {
		const page = Math.max(1, Math.min(5, Math.floor(request.page ?? 1) || 1));
		const diagnostics = {
			baseUrl,
			page
		};
		const url = new URL(`${baseUrl}/search`);
		url.searchParams.set("q", request.query.trim());
		url.searchParams.set("format", "json");
		url.searchParams.set("pageno", String(page));
		const enabled = this.engines.filter((engine) => !this.disabledEngines.includes(engine));
		if (this.engines.length && !enabled.length) throw new SearXNGError("All requested SearXNG engines are disabled.", diagnostics, false);
		if (enabled.length) url.searchParams.set("engines", enabled.join(","));
		if (this.disabledEngines.length) url.searchParams.set("disabled_engines", this.disabledEngines.join(","));
		if (request.language) url.searchParams.set("language", request.language);
		if (request.category) url.searchParams.set("categories", request.category);
		if (request.freshness && request.freshness !== "any") url.searchParams.set("time_range", request.freshness === "week" ? "month" : request.freshness === "realtime" ? "day" : request.freshness);
		const controller = new AbortController();
		const timer = setTimeout(() => controller.abort(), Math.max(1, Math.floor(timeoutMs)));
		try {
			const res = await fetch(url.toString(), {
				headers: { Accept: "application/json" },
				signal: controller.signal,
				redirect: "error"
			});
			diagnostics.httpStatus = res.status;
			if (!res.ok) {
				let message = `SearXNG returned HTTP ${res.status}.`;
				if (res.status === 403) message += " Enable json under search.formats in SearXNG settings.yml and check access restrictions.";
				if (res.status === 429 || res.status === 503) {
					const value = res.headers.get("Retry-After")?.trim();
					const parsed = value && /^\d+$/.test(value) ? Date.now() + Number(value) * 1e3 : Date.parse(value || "");
					this.retryAfter.set(baseUrl, Number.isFinite(parsed) ? Math.max(Date.now(), parsed) : Date.now() + 3e4);
					message += " Retry-After cooldown applied to this instance.";
				}
				throw new SearXNGError(message, diagnostics, res.status >= 500 && res.status !== 503);
			}
			let data;
			try {
				data = await res.json();
			} catch (error) {
				if (controller.signal.aborted) throw error;
				throw new SearXNGError("SearXNG did not return valid JSON. Enable json under search.formats in settings.yml and check the base URL.", diagnostics, false);
			}
			if (!data || !Array.isArray(data.results) || data.error) throw new SearXNGError(`SearXNG returned an invalid search response${data?.error ? `: ${String(data.error).slice(0, 300)}` : ": missing results array"}.`, diagnostics, false);
			diagnostics.engines = engineFailures(data);
			diagnostics.unresponsiveEngines = Object.keys(diagnostics.engines);
			const meta = normalizeSearxMeta(data);
			const limit = Math.max(1, Math.min(50, Math.floor(request.maxResults ?? 10) || 10));
			const results = [];
			for (let i = 0; i < data.results.length && results.length < limit; i++) {
				const raw = data.results[i];
				if (!raw || typeof raw !== "object" || !usableMetaUrl(raw.url)) continue;
				results.push(normalizeRawSearchResult(raw, request.query, i + 1, page));
			}
			if (!results.length && !meta.answers.length && !meta.infoboxes.length && diagnostics.unresponsiveEngines.length) throw new SearXNGError(`SearXNG returned no usable results; engine failures: ${Object.entries(diagnostics.engines).map(([name, reason]) => `${name}: ${reason}`).join("; ")}`, diagnostics);
			return {
				results,
				meta,
				diagnostics
			};
		} catch (error) {
			if (controller.signal.aborted) throw new SearXNGError(`SearXNG request timed out after ${this.timeoutMs}ms (instance budget ${Math.floor(timeoutMs)}ms).`, {
				...diagnostics,
				error: "timeout"
			});
			const failure = error instanceof SearXNGError ? error : new SearXNGError(`Could not reach SearXNG at ${baseUrl}: ${error instanceof Error ? error.message : String(error)}`, diagnostics);
			failure.diagnostics.error = failure.message;
			throw failure;
		} finally {
			clearTimeout(timer);
		}
	}
};
//#endregion
//#region src/web/search/search_service.ts
var SearchService = class {
	provider;
	maxConcurrent;
	retries;
	retryDelayMs;
	constructor(provider, options = {}) {
		this.provider = provider;
		this.maxConcurrent = Math.max(1, Math.min(8, Math.floor(options.maxConcurrentQueries ?? 4) || 4));
		this.retries = Math.max(0, Math.min(3, Math.floor(options.retries ?? 1) || 0));
		this.retryDelayMs = Math.max(0, Math.min(1e4, options.retryDelayMs ?? 1e3));
	}
	async searchOnce(query, maxResults) {
		let lastError = "";
		for (let attempt = 0; attempt <= this.retries; attempt++) try {
			return await this.provider.search({
				query: query.query,
				freshness: query.freshness,
				maxResults,
				page: query.page,
				language: query.language,
				category: query.category
			});
		} catch (err) {
			lastError = String(err?.message || err);
			if (err?.retryable === false || /retry-after/i.test(lastError)) break;
			if (attempt < this.retries) await new Promise((r) => setTimeout(r, this.retryDelayMs));
		}
		throw new Error(lastError || "search failed");
	}
	async executeSearches(queries, maxResultsPerQuery = 10) {
		if (queries.length === 0) return {
			results: [],
			rawCount: 0,
			failureCount: 0,
			failures: [],
			meta: emptySearchMeta()
		};
		const successfulGroups = [];
		const failures = [];
		let rawCount = 0;
		let meta = emptySearchMeta();
		const queue = [...queries];
		const workers = Array.from({ length: Math.min(this.maxConcurrent, queue.length) }, async () => {
			while (queue.length > 0) {
				const q = queue.shift();
				try {
					const hits = await this.searchOnce(q, maxResultsPerQuery);
					successfulGroups.push({
						query: q.query,
						results: hits
					});
					rawCount += hits.length;
					const attached = metaForResults(hits);
					if (attached) meta = mergeSearchMeta(meta, attached);
				} catch (err) {
					failures.push({
						query: q.query,
						error: String(err?.message || err)
					});
				}
			}
		});
		await Promise.all(workers);
		return {
			results: fuseSearchResults(successfulGroups),
			rawCount,
			failureCount: failures.length,
			failures,
			meta
		};
	}
	async fetchAdditionalPage(query, page, maxResultsPerQuery = 10) {
		try {
			const hits = await this.searchOnce({
				...query,
				page
			}, maxResultsPerQuery);
			return {
				results: hits,
				meta: metaForResults(hits)
			};
		} catch (err) {
			return {
				results: [],
				error: String(err?.message || err)
			};
		}
	}
};
//#endregion
//#region src/web/search/google_provider.ts
var GoogleSearchProvider = class {
	apiKey;
	cxId;
	timeoutMs;
	constructor(apiKey, cxId, timeoutMs = 8e3) {
		this.apiKey = apiKey;
		this.cxId = cxId;
		this.timeoutMs = timeoutMs;
	}
	async search(request) {
		const url = new URL("https://www.googleapis.com/customsearch/v1");
		url.searchParams.set("key", this.apiKey);
		url.searchParams.set("cx", this.cxId);
		url.searchParams.set("q", request.query);
		url.searchParams.set("num", String(Math.min(request.maxResults ?? 10, 10)));
		if (request.language) url.searchParams.set("lr", `lang_${request.language}`);
		const controller = new AbortController();
		const timer = setTimeout(() => controller.abort(), this.timeoutMs);
		try {
			const res = await fetch(url.toString(), {
				headers: { Accept: "application/json" },
				signal: controller.signal,
				redirect: "error"
			});
			if (!res.ok) {
				const body = await res.text();
				throw new Error(`Google Search returned HTTP ${res.status}: ${body.slice(0, 200)}`);
			}
			const data = await res.json();
			clearTimeout(timer);
			if (!data || !Array.isArray(data.items)) return [];
			const limit = request.maxResults ?? 10;
			const results = [];
			for (let i = 0; i < Math.min(data.items.length, limit); i++) {
				const item = data.items[i];
				results.push({
					id: `google-${i + 1}`,
					queryId: request.query,
					title: item.title || "",
					url: item.link || "",
					snippet: item.snippet || "",
					domain: "",
					publishedAt: void 0,
					engine: "google",
					rank: i + 1,
					score: void 0,
					metadata: {}
				});
			}
			return results;
		} catch (err) {
			clearTimeout(timer);
			if (err.name === "AbortError") throw new Error(`Google Search request timed out after ${this.timeoutMs}ms`);
			throw err;
		}
	}
};
//#endregion
//#region src/web/fetch/http_fetcher.ts
var HttpFetcher = class {
	globalConcurrency;
	perDomainConcurrency;
	transport;
	constructor(globalConcurrency = 8, perDomainConcurrency = 2) {
		this.globalConcurrency = globalConcurrency;
		this.perDomainConcurrency = perDomainConcurrency;
	}
	async fetch(url, options = {}) {
		try {
			this.transport ??= import(
				/* @vite-ignore */
				"../../../scripts/web-transport.mjs"
).then((module) => new module.PinnedPageFetcher(this.globalConcurrency, this.perDomainConcurrency));
			return await (await this.transport).fetch(url, options);
		} catch (error) {
			return {
				url,
				finalUrl: url,
				success: false,
				durationMs: 0,
				error: `Native retrieval unavailable: ${String(error)}`
			};
		}
	}
};
//#endregion
//#region src/web/fetch/github_fast_path.ts
/**
* GitHub fast path: github.com pages are heavy JS shells that the raw,
* no-render fetcher extracts poorly, while the underlying content is freely
* available as plain text. Blob URLs map deterministically onto
* raw.githubusercontent.com; repository roots map onto the README at the
* default branch. Failures fall back to the ordinary HTML fetch, so the fast
* path can only add content, never remove it.
*/
var GITHUB_HOST = "github.com";
var RAW_HOST = "https://raw.githubusercontent.com";
function safeSegment(value) {
	return value.length > 0 && !value.includes("\\") && !/^\.+$/.test(value);
}
/** Map a github.com URL onto its raw content URL(s); null when not applicable. */
function toGitHubRawCandidates(url) {
	let parsed;
	try {
		parsed = new URL(url);
	} catch {
		return null;
	}
	if (parsed.hostname !== GITHUB_HOST || parsed.protocol !== "https:") return null;
	const segments = parsed.pathname.split("/").filter((s) => s.length > 0);
	if (segments.length < 2 || !segments.every(safeSegment)) return null;
	if ((/* @__PURE__ */ new Set([
		"settings",
		"topics",
		"orgs",
		"organizations",
		"marketplace",
		"pulls",
		"issues",
		"notifications",
		"explore",
		"trending",
		"features",
		"security",
		"pricing",
		"sponsors",
		"collections"
	])).has(segments[0].toLowerCase())) return null;
	const [owner, repo, third, fourth, ...rest] = segments;
	if (segments.length === 2) return [
		{
			rawUrl: `${RAW_HOST}/${owner}/${repo}/HEAD/README.md`,
			kind: "readme"
		},
		{
			rawUrl: `${RAW_HOST}/${owner}/${repo}/main/README.md`,
			kind: "readme"
		},
		{
			rawUrl: `${RAW_HOST}/${owner}/${repo}/master/README.md`,
			kind: "readme"
		}
	];
	if (third === "blob" && fourth && rest.length >= 0) return [{
		rawUrl: `${RAW_HOST}/${owner}/${repo}/${[fourth, ...rest].map(encodeURIComponent).join("/")}`,
		kind: "blob"
	}];
	return null;
}
/** True when a fast-path candidate response actually carries usable content. */
function isUsableRawResponse(status, mime, body, kind) {
	if (status !== 200 || !body) return false;
	if (kind === "blob") return body.trim().length > 0;
	return (!mime || /^(text\/(plain|markdown)|application\/octet-stream)/.test(mime)) && body.trim().length > 60;
}
//#endregion
//#region src/web/fetch/wayback.ts
/**
* Wayback Machine fallback: recovers a failed page fetch from the Internet
* Archive. Only consulted after a live fetch fails or returns no usable body,
* so it costs nothing on healthy pages. The CDX availability lookup and the
* snapshot download both travel through the same pinned, SSRF-checked fetcher;
* archive.org permits automated fetching (its robots.txt only disallows
* /control/ and /report/) and web.archive.org publishes no robots.txt.
*/
var CDX_URL = "https://archive.org/wayback/available";
var CDX_TIMEOUT_MS = 8e3;
function isHttpUrl(url) {
	try {
		return /^https?:$/.test(new URL(url).protocol);
	} catch {
		return false;
	}
}
/** Look up the closest archived snapshot for a URL. Returns null on any miss. */
async function queryWaybackSnapshot(url, fetcher) {
	if (!isHttpUrl(url)) return null;
	try {
		const res = await fetcher.fetch(`${CDX_URL}?url=${encodeURIComponent(url)}`, {
			timeoutSeconds: Math.ceil(CDX_TIMEOUT_MS / 1e3),
			maxBytes: 262144
		});
		if (!res.success || !res.body) return null;
		const closest = JSON.parse(res.body).archived_snapshots?.closest;
		if (!closest?.url || closest.available === false) return null;
		if (!closest.url.startsWith("https://web.archive.org/")) return null;
		const ts = closest.timestamp ?? "";
		const archivedAt = /^\d{8}/.test(ts) ? `${ts.slice(0, 4)}-${ts.slice(4, 6)}-${ts.slice(6, 8)}` : "unknown date";
		return {
			snapshotUrl: closest.url,
			archivedAt
		};
	} catch {
		return null;
	}
}
/**
* Attempt full recovery of a URL from the Wayback Machine: snapshot lookup,
* bounded download, main-content extraction, and an explicit provenance line
* prepended to the text so downstream synthesis knows this is an archived copy.
*/
async function fetchViaWayback(originalUrl, fetcher, options = {}) {
	const snapshot = await queryWaybackSnapshot(originalUrl, fetcher);
	if (!snapshot) return null;
	try {
		const res = await fetcher.fetch(snapshot.snapshotUrl, {
			timeoutSeconds: options.timeoutSeconds,
			maxBytes: options.maxBytes
		});
		if (!res.success || !res.body || res.body.length < 150) return null;
		const extraction = extractMainContent(res.body, options.titleHint || originalUrl);
		if (!extraction.text || extraction.text.trim().length < 50) return null;
		const provenance = `> [via Wayback Machine, archived ${snapshot.archivedAt}] — live page was unavailable\n\n`;
		return {
			originalUrl,
			title: `[Archived] ${extraction.title || options.titleHint || originalUrl}`,
			text: provenance + extraction.text,
			snapshotUrl: res.finalUrl || snapshot.snapshotUrl,
			archivedAt: snapshot.archivedAt
		};
	} catch {
		return null;
	}
}
//#endregion
//#region src/web/fetch/challenge.ts
var CHALLENGE_MISS_REASON = "challenge_detected";
var BODY_SCAN_LIMIT = 65536;
var BODY_MARKERS = [
	{
		marker: "title:just-a-moment",
		re: /<title[^>]*>\s*just a moment/i
	},
	{
		marker: "cf-chl",
		re: /cf-chl/i
	},
	{
		marker: "challenge-platform",
		re: /challenge-platform/i
	},
	{
		marker: "_cf_chl_opt",
		re: /_cf_chl_opt/i
	}
];
function detectChallenge(status, body) {
	if (status === 403 || status === 503) {
		if (body) {
			const head = body.slice(0, BODY_SCAN_LIMIT);
			for (const { marker, re } of BODY_MARKERS) if (re.test(head)) return {
				kind: "status_headers",
				marker: `status_${status}:${marker}`
			};
		}
	}
	if (body) {
		const head = body.slice(0, BODY_SCAN_LIMIT);
		for (const { marker, re } of BODY_MARKERS) if (re.test(head)) return {
			kind: "interstitial_body",
			marker
		};
	}
	return null;
}
//#endregion
//#region src/web/fetch/cascade.ts
/**
* In-process fetch cascade modeled on self-hosted SearXNG stacks:
* GitHub raw content → live HTML fetch → Wayback Machine.
*
* No headless browser, Firecrawl, or Crawl4AI: those need extra containers
* and RAM that a local-model desktop harness should not require. Each stage
* is optional and fail-soft. Challenge interstitials are treated as misses.
*/
function domainOf(url) {
	try {
		return new URL(url).hostname.toLowerCase();
	} catch {
		return "";
	}
}
function isPdf(res, url) {
	return (res.mimeType || "").split(";")[0].trim().toLowerCase() === "application/pdf" || /\.pdf(\?|#|$)/i.test(res.finalUrl || url);
}
function fromHtml(body, titleHint, finalUrl, method) {
	const extraction = extractMainContent(body, titleHint);
	return {
		title: extraction.title || titleHint,
		text: extraction.text,
		finalUrl,
		method,
		publishedAt: extraction.publishedAt,
		author: extraction.author,
		canonicalUrl: extraction.canonicalUrl,
		headings: extraction.headings,
		links: extraction.links,
		contentHash: extraction.contentHash
	};
}
async function fetchPageCascade(url, fetcher, options = {}) {
	const domain = domainOf(url);
	const fetchOpts = {
		timeoutSeconds: options.timeoutSeconds,
		maxBytes: options.maxBytes,
		userAgent: options.userAgent
	};
	const titleHint = options.titleHint || url;
	let skippedLive = false;
	if (options.domainStats && domain) {
		if ((await options.domainStats.status(domain)).chronic) skippedLive = true;
	}
	const record = async (ok, error) => {
		if (options.domainStats && domain) await options.domainStats.record(domain, ok, error);
	};
	let failureDetail;
	const tryRender = async () => {
		if (!options.jsRender) return null;
		const rendered = await options.jsRender(url).catch(() => null);
		if (!rendered?.html || detectChallenge(200, rendered.html)) return null;
		const document = fromHtml(rendered.html, titleHint, rendered.finalUrl || url, "js_render");
		if (!document.text.trim()) return null;
		return {
			document,
			method: "js_render"
		};
	};
	const minUsefulChars = options.jsRenderMinChars ?? 280;
	if (!skippedLive) {
		const github = toGitHubRawCandidates(url);
		if (github) for (const candidate of github) {
			const res = await fetcher.fetch(candidate.rawUrl, fetchOpts);
			if (isUsableRawResponse(res.status, res.mimeType, res.body, candidate.kind)) {
				await record(true);
				return {
					document: fromHtml(res.body, titleHint, res.finalUrl || candidate.rawUrl, "github_raw"),
					raw: res,
					method: "github_raw"
				};
			}
		}
		const live = await fetcher.fetch(url, fetchOpts);
		const mime = (live.mimeType || "").split(";")[0].trim().toLowerCase();
		if (live.success && typeof live.body === "string" && ([
			"application/json",
			"text/csv",
			"application/csv",
			"text/tab-separated-values"
		].includes(mime) || mime.endsWith("+json"))) {
			await record(true);
			return {
				document: null,
				raw: live,
				method: "live"
			};
		}
		if (live.success && live.body && live.body.length >= 150) {
			if (isPdf(live, url)) {
				await record(true);
				return {
					document: null,
					raw: live,
					method: "live"
				};
			}
			const challenge = detectChallenge(live.status ?? 200, live.body);
			if (challenge) {
				failureDetail = `${CHALLENGE_MISS_REASON}:${challenge.marker}`;
				await record(false, failureDetail);
				const rendered = await tryRender();
				if (rendered) {
					await record(true);
					return rendered;
				}
			} else {
				const document = fromHtml(live.body, titleHint, live.finalUrl || url, "live");
				if (options.jsRender && document.text.trim().length < minUsefulChars) {
					const rendered = await tryRender();
					if (rendered && rendered.document.text.trim().length > document.text.trim().length) {
						await record(true);
						return rendered;
					}
				}
				await record(true);
				return {
					document,
					raw: live,
					method: "live"
				};
			}
		} else {
			failureDetail = live.error || "empty or thin response";
			await record(false, live.error);
			if (!/robots\.txt/i.test(live.error || "")) {
				const rendered = await tryRender();
				if (rendered) {
					await record(true);
					return rendered;
				}
			}
		}
	} else {
		const rendered = await tryRender();
		if (rendered) {
			await record(true);
			return rendered;
		}
	}
	if (options.waybackFallback !== false) {
		const recovered = await fetchViaWayback(url, fetcher, {
			timeoutSeconds: options.timeoutSeconds,
			maxBytes: options.maxBytes,
			titleHint
		});
		if (recovered) return {
			document: {
				title: recovered.title,
				text: recovered.text,
				finalUrl: recovered.snapshotUrl,
				method: "wayback",
				contentHash: computeContentHash(recovered.text),
				archivedAt: recovered.archivedAt
			},
			method: "wayback",
			skippedLive
		};
	}
	return {
		document: null,
		method: "failed",
		skippedLive,
		error: skippedLive ? "chronic domain skipped live fetch" : failureDetail || "fetch failed"
	};
}
//#endregion
//#region src/web/fetch/domain_stats.ts
var DOMAIN_RECORD_TTL_SECONDS = 7776e3;
var DomainStatsStore = class {
	storage;
	options;
	constructor(storage, options = {}) {
		this.storage = storage;
		this.options = options;
	}
	get windowMs() {
		return this.options.windowMs ?? 2592e6;
	}
	get minAttempts() {
		return Math.max(2, this.options.minAttempts ?? 4);
	}
	get chronicFailRate() {
		return this.options.chronicFailRate ?? .7;
	}
	/** Current-window view of a stored stat: an expired window reads as empty. */
	currentWindow(stat) {
		if (!stat || typeof stat.attempts !== "number") return {
			attempts: 0,
			ok: 0,
			fail: 0,
			windowStartMs: Date.now()
		};
		if (Date.now() - stat.windowStartMs <= this.windowMs) return stat;
		return {
			attempts: 0,
			ok: 0,
			fail: 0,
			windowStartMs: stat.windowStartMs
		};
	}
	async record(domain, ok, error) {
		const key = domain.toLowerCase();
		if (!key) return;
		try {
			const stored = await this.storage.get("domain_stats", key);
			const stat = this.currentWindow(stored);
			stat.attempts += 1;
			if (ok) stat.ok += 1;
			else {
				stat.fail += 1;
				if (error) stat.lastError = String(error).slice(0, 200);
			}
			stat.lastAttemptAt = (/* @__PURE__ */ new Date()).toISOString();
			await this.storage.set("domain_stats", key, stat, DOMAIN_RECORD_TTL_SECONDS);
		} catch {}
	}
	async status(domain) {
		const key = domain.toLowerCase();
		try {
			const stat = this.currentWindow(await this.storage.get("domain_stats", key));
			const failureRate = stat.attempts > 0 ? stat.fail / stat.attempts : null;
			return {
				attempts: stat.attempts,
				ok: stat.ok,
				fail: stat.fail,
				failureRate,
				chronic: stat.attempts >= this.minAttempts && failureRate !== null && failureRate >= this.chronicFailRate
			};
		} catch {
			return {
				attempts: 0,
				ok: 0,
				fail: 0,
				failureRate: null,
				chronic: false
			};
		}
	}
};
//#endregion
//#region node_modules/fflate/esm/index.mjs
var require = createRequire("/");
var _a;
try {
	_a = require("worker_threads"), _a.Worker, _a.isMarkedAsUntransferable;
} catch (e) {}
var u8 = Uint8Array;
var u16 = Uint16Array;
var i32 = Int32Array;
var fleb = new u8([
	0,
	0,
	0,
	0,
	0,
	0,
	0,
	0,
	1,
	1,
	1,
	1,
	2,
	2,
	2,
	2,
	3,
	3,
	3,
	3,
	4,
	4,
	4,
	4,
	5,
	5,
	5,
	5,
	0,
	0,
	0,
	0
]);
var fdeb = new u8([
	0,
	0,
	0,
	0,
	1,
	1,
	2,
	2,
	3,
	3,
	4,
	4,
	5,
	5,
	6,
	6,
	7,
	7,
	8,
	8,
	9,
	9,
	10,
	10,
	11,
	11,
	12,
	12,
	13,
	13,
	0,
	0
]);
var clim = new u8([
	16,
	17,
	18,
	0,
	8,
	7,
	9,
	6,
	10,
	5,
	11,
	4,
	12,
	3,
	13,
	2,
	14,
	1,
	15
]);
var freb = function(eb, start) {
	var b = new u16(31);
	for (var i = 0; i < 31; ++i) b[i] = start += 1 << eb[i - 1];
	var r = new i32(b[30]);
	for (var i = 1; i < 30; ++i) for (var j = b[i]; j < b[i + 1]; ++j) r[j] = j - b[i] << 5 | i;
	return {
		b,
		r
	};
};
var _a = freb(fleb, 2);
var fl = _a.b;
var revfl = _a.r;
fl[28] = 258, revfl[258] = 28;
var _b = freb(fdeb, 0);
var fd = _b.b;
_b.r;
var rev = new u16(32768);
for (var i = 0; i < 32768; ++i) {
	var x = (i & 43690) >> 1 | (i & 21845) << 1;
	x = (x & 52428) >> 2 | (x & 13107) << 2;
	x = (x & 61680) >> 4 | (x & 3855) << 4;
	rev[i] = ((x & 65280) >> 8 | (x & 255) << 8) >> 1;
}
var hMap = (function(cd, mb, r) {
	var s = cd.length;
	var i = 0;
	var l = new u16(mb);
	for (; i < s; ++i) if (cd[i]) ++l[cd[i] - 1];
	var le = new u16(mb);
	for (i = 1; i < mb; ++i) le[i] = le[i - 1] + l[i - 1] << 1;
	var co;
	if (r) {
		co = new u16(1 << mb);
		var rvb = 15 - mb;
		for (i = 0; i < s; ++i) if (cd[i]) {
			var sv = i << 4 | cd[i];
			var r_1 = mb - cd[i];
			var v = le[cd[i] - 1]++ << r_1;
			for (var m = v | (1 << r_1) - 1; v <= m; ++v) co[rev[v] >> rvb] = sv;
		}
	} else {
		co = new u16(s);
		for (i = 0; i < s; ++i) if (cd[i]) co[i] = rev[le[cd[i] - 1]++] >> 15 - cd[i];
	}
	return co;
});
var flt = new u8(288);
for (var i = 0; i < 144; ++i) flt[i] = 8;
for (var i = 144; i < 256; ++i) flt[i] = 9;
for (var i = 256; i < 280; ++i) flt[i] = 7;
for (var i = 280; i < 288; ++i) flt[i] = 8;
var fdt = new u8(32);
for (var i = 0; i < 32; ++i) fdt[i] = 5;
var flrm = /*#__PURE__*/ hMap(flt, 9, 1);
var fdrm = /*#__PURE__*/ hMap(fdt, 5, 1);
var max = function(a) {
	var m = a[0];
	for (var i = 1; i < a.length; ++i) if (a[i] > m) m = a[i];
	return m;
};
var bits = function(d, p, m) {
	var o = p / 8 | 0;
	return (d[o] | d[o + 1] << 8) >> (p & 7) & m;
};
var bits16 = function(d, p) {
	var o = p / 8 | 0;
	return (d[o] | d[o + 1] << 8 | d[o + 2] << 16) >> (p & 7);
};
var shft = function(p) {
	return (p + 7) / 8 | 0;
};
var slc = function(v, s, e) {
	if (s == null || s < 0) s = 0;
	if (e == null || e > v.length) e = v.length;
	return new u8(v.subarray(s, e));
};
var ec = [
	"unexpected EOF",
	"invalid block type",
	"invalid length/literal",
	"invalid distance",
	"stream finished",
	"no stream handler",
	,
	"no callback",
	"invalid UTF-8 data",
	"extra field too long",
	"date not in range 1980-2099",
	"filename too long",
	"stream finishing",
	"invalid zip data"
];
var err = function(ind, msg, nt) {
	var e = new Error(msg || ec[ind]);
	e.code = ind;
	if (Error.captureStackTrace) Error.captureStackTrace(e, err);
	if (!nt) throw e;
	return e;
};
var inflt = function(dat, st, buf, dict) {
	var sl = dat.length, dl = dict ? dict.length : 0;
	if (!sl || st.f && !st.l) return buf || new u8(0);
	var noBuf = !buf;
	var resize = noBuf || st.i != 2;
	var noSt = st.i;
	if (noBuf) buf = new u8(sl * 3);
	var cbuf = function(l) {
		var bl = buf.length;
		if (l > bl) {
			var nbuf = new u8(Math.max(bl * 2, l));
			nbuf.set(buf);
			buf = nbuf;
		}
	};
	var final = st.f || 0, pos = st.p || 0, bt = st.b || 0, lm = st.l, dm = st.d, lbt = st.m, dbt = st.n;
	var tbts = sl * 8;
	do {
		if (!lm) {
			final = bits(dat, pos, 1);
			var type = bits(dat, pos + 1, 3);
			pos += 3;
			if (!type) {
				var s = shft(pos) + 4, l = dat[s - 4] | dat[s - 3] << 8, t = s + l;
				if (t > sl) {
					if (noSt) err(0);
					break;
				}
				if (resize) cbuf(bt + l);
				buf.set(dat.subarray(s, t), bt);
				st.b = bt += l, st.p = pos = t * 8, st.f = final;
				continue;
			} else if (type == 1) lm = flrm, dm = fdrm, lbt = 9, dbt = 5;
			else if (type == 2) {
				var hLit = bits(dat, pos, 31) + 257, hcLen = bits(dat, pos + 10, 15) + 4;
				var tl = hLit + bits(dat, pos + 5, 31) + 1;
				pos += 14;
				var ldt = new u8(tl);
				var clt = new u8(19);
				for (var i = 0; i < hcLen; ++i) clt[clim[i]] = bits(dat, pos + i * 3, 7);
				pos += hcLen * 3;
				var clb = max(clt), clbmsk = (1 << clb) - 1;
				var clm = hMap(clt, clb, 1);
				for (var i = 0; i < tl;) {
					var r = clm[bits(dat, pos, clbmsk)];
					pos += r & 15;
					var s = r >> 4;
					if (s < 16) ldt[i++] = s;
					else {
						var c = 0, n = 0;
						if (s == 16) n = 3 + bits(dat, pos, 3), pos += 2, c = ldt[i - 1];
						else if (s == 17) n = 3 + bits(dat, pos, 7), pos += 3;
						else if (s == 18) n = 11 + bits(dat, pos, 127), pos += 7;
						while (n--) ldt[i++] = c;
					}
				}
				var lt = ldt.subarray(0, hLit), dt = ldt.subarray(hLit);
				lbt = max(lt);
				dbt = max(dt);
				lm = hMap(lt, lbt, 1);
				dm = hMap(dt, dbt, 1);
			} else err(1);
			if (pos > tbts) {
				if (noSt) err(0);
				break;
			}
		}
		if (resize) cbuf(bt + 131072);
		var lms = (1 << lbt) - 1, dms = (1 << dbt) - 1;
		var lpos = pos;
		for (;; lpos = pos) {
			var c = lm[bits16(dat, pos) & lms], sym = c >> 4;
			pos += c & 15;
			if (pos > tbts) {
				if (noSt) err(0);
				break;
			}
			if (!c) err(2);
			if (sym < 256) buf[bt++] = sym;
			else if (sym == 256) {
				lpos = pos, lm = null;
				break;
			} else {
				var add = sym - 254;
				if (sym > 264) {
					var i = sym - 257, b = fleb[i];
					add = bits(dat, pos, (1 << b) - 1) + fl[i];
					pos += b;
				}
				var d = dm[bits16(dat, pos) & dms], dsym = d >> 4;
				if (!d) err(3);
				pos += d & 15;
				var dt = fd[dsym];
				if (dsym > 3) {
					var b = fdeb[dsym];
					dt += bits16(dat, pos) & (1 << b) - 1, pos += b;
				}
				if (pos > tbts) {
					if (noSt) err(0);
					break;
				}
				if (resize) cbuf(bt + 131072);
				var end = bt + add;
				if (bt < dt) {
					var shift = dl - dt, dend = Math.min(dt, end);
					if (shift + bt < 0) err(3);
					for (; bt < dend; ++bt) buf[bt] = dict[shift + bt];
				}
				for (; bt < end; ++bt) buf[bt] = buf[bt - dt];
			}
		}
		st.l = lm, st.p = lpos, st.b = bt, st.f = final;
		if (lm) final = 1, st.m = lbt, st.d = dm, st.n = dbt;
	} while (!final);
	return bt != buf.length && noBuf ? slc(buf, 0, bt) : buf.subarray(0, bt);
};
var et = /*#__PURE__*/ new u8(0);
var zls = function(d, dict) {
	if ((d[0] & 15) != 8 || d[0] >> 4 > 7 || (d[0] << 8 | d[1]) % 31) err(6, "invalid zlib data");
	if ((d[1] >> 5 & 1) == +!dict) err(6, "invalid zlib data: " + (d[1] & 32 ? "need" : "unexpected") + " dictionary");
	return (d[1] >> 3 & 4) + 2;
};
function inflateSync(data, opts) {
	return inflt(data, { i: 2 }, opts && opts.out, opts && opts.dictionary);
}
function unzlibSync(data, opts) {
	return inflt(data.subarray(zls(data, opts && opts.dictionary), -4), { i: 2 }, opts && opts.out, opts && opts.dictionary);
}
var td = typeof TextDecoder != "undefined" && /*#__PURE__*/ new TextDecoder();
try {
	td.decode(et, { stream: true });
} catch (e) {}
//#endregion
//#region src/web/chunking/tokenizer.ts
var ApproximateTokenCounter = class {
	count(text) {
		if (!text) return 0;
		const words = text.trim().split(/\s+/).filter(Boolean);
		const charEstimate = Math.ceil(text.length / 3.8);
		const wordEstimate = Math.ceil(words.length * 1.33);
		return Math.max(charEstimate, wordEstimate);
	}
};
var defaultTokenCounter = new ApproximateTokenCounter();
//#endregion
//#region src/web/documents/document_store.ts
/**
* Document Store: full stored document text with stable evidence references.
*
* The worker keeps full documents outside model context (SQLite `research`
* rows) and serves bounded passages with stable `docId:passage` references.
* Open/find tools read from this store — never only from previously
* compressed evidence — so facts excluded from the initial summary stay
* recoverable. PDF text carries page references. Missing text is
* `no_extractable_text`; it is not labeled OCR-required.
*/
var MAX_PASSAGE_CHARS = 2e3;
var MAX_PASSAGES = 5;
var DocumentStore = class {
	docs = /* @__PURE__ */ new Map();
	save(doc) {
		this.docs.set(doc.id, {
			...doc,
			fullText: doc.fullText || doc.text
		});
	}
	get(docId) {
		return this.docs.get(docId);
	}
	ids() {
		return [...this.docs.keys()];
	}
	/** Read exact windows of stored text. Cursors never skip a long paragraph. */
	open(docId, options = {}) {
		const doc = this.docs.get(docId);
		if (!doc) throw new Error("Unknown document in this session");
		let fullText = doc.fullText || doc.text;
		if (!fullText.trim()) throw new Error(`Document ${docId} has no stored text`);
		let page;
		if (typeof options.page === "number" && doc.pages?.length) {
			const index = Math.max(1, Math.min(doc.pages.length, Math.floor(options.page))) - 1;
			page = index + 1;
			fullText = doc.pages[index];
		}
		let section;
		let start = Math.max(0, Math.floor(options.textOffset ?? ((options.passage ?? options.offset ?? 1) - 1) * MAX_PASSAGE_CHARS));
		if (options.section && options.textOffset == null) {
			const hit = [...fullText.matchAll(/^#{1,6}\s+(.+)$/gm)].find((match) => match[1].toLowerCase().includes(options.section.toLowerCase()));
			if (!hit) throw new Error("Unknown section in this document");
			start = hit.index;
			section = hit[1];
		}
		if (start >= fullText.length) return [];
		if (start > 0 && /[\uDC00-\uDFFF]/.test(fullText[start])) start--;
		let end = Math.min(fullText.length, start + MAX_PASSAGE_CHARS);
		if (end < fullText.length && /[\uDC00-\uDFFF]/.test(fullText[end])) end--;
		return [{
			ref: page ? `${docId}:page:${page}${start ? `:offset:${start}` : ""}` : `${docId}:passage:${Math.floor(start / MAX_PASSAGE_CHARS) + 1}:offset:${start}`,
			docId,
			page,
			section,
			text: fullText.slice(start, end),
			chars: end - start,
			textOffset: start,
			nextTextOffset: end < fullText.length ? end : null,
			totalChars: fullText.length
		}];
	}
	/** Document-local search over full stored text with surrounding context. */
	find(docId, term, contextChars = 400) {
		const doc = this.docs.get(docId);
		if (!doc) throw new Error("Unknown document in this session");
		const fullText = doc.fullText || doc.text;
		const needle = term.toLowerCase();
		if (!needle.trim()) throw new Error("Search term must not be empty");
		const out = [];
		let from = 0;
		let hitIndex = 0;
		while (out.length < MAX_PASSAGES) {
			const at = fullText.toLowerCase().indexOf(needle, from);
			if (at < 0) break;
			hitIndex++;
			const start = Math.max(0, at - contextChars);
			const end = Math.min(fullText.length, at + needle.length + contextChars);
			out.push({
				ref: `${docId}:match:${hitIndex}`,
				docId,
				text: fullText.slice(start, end),
				chars: end - start,
				textOffset: start,
				nextTextOffset: end < fullText.length ? end : null,
				totalChars: fullText.length
			});
			from = at + needle.length;
		}
		return out;
	}
	passageStats() {
		let chars = 0;
		for (const doc of this.docs.values()) chars += (doc.fullText || doc.text).length;
		return {
			documents: this.docs.size,
			chars,
			tokens: defaultTokenCounter.count([...this.docs.values()].map((d) => d.fullText || d.text).join("\n"))
		};
	}
};
var PDF_BYTE_LIMIT = 8e6;
var PDF_INFLATE_LIMIT = 2e6;
var PdfLimitExceeded = class extends Error {
	constructor() {
		super("PDF inflate limit exceeded");
		this.name = "PdfLimitExceeded";
	}
};
/** Local PDF text extraction.
*  Bytes are parsed in-process. `title` is ignored so a remote URL is never
*  fetched or parsed, and external PDF resources are not loaded.
*  `needsOcr` stays false: little or missing text is not labeled OCR-required. */
function extractPdfText(bytes, title = "") {
	if (bytes.length > PDF_BYTE_LIMIT) return pdfResult("limit_exceeded");
	const raw = bytesToLatin1(bytes);
	if (/\/Encrypt\s*(?:\d+\s+\d+\s+R|<<)/.test(raw)) return pdfResult("password_required");
	if (!looksLikePdf(raw)) return pdfResult("invalid_document");
	try {
		const budget = { used: 0 };
		const pages = pageBodies(raw).map((body) => normalizePdfText(pageText(body, budget)));
		const text = normalizePdfText(pages.join("\n"));
		return {
			pages,
			text,
			needsOcr: false,
			status: text.length > 0 ? "text_extracted" : "no_extractable_text"
		};
	} catch (error) {
		if (error instanceof PdfLimitExceeded) return pdfResult("limit_exceeded");
		throw error;
	}
}
function pdfResult(status) {
	return {
		pages: [],
		text: "",
		needsOcr: false,
		status
	};
}
function looksLikePdf(raw) {
	const hasPage = /\/Type\s*\/Page(?!s)/.test(raw);
	const hasTextOp = /(?:^|[^A-Za-z])(?:Tj|TJ)(?![A-Za-z])/.test(raw);
	if (hasPage || hasTextOp) return true;
	if (!raw.includes("%PDF")) return false;
	return /\b\d+\s+\d+\s+obj\b/.test(raw) || /(?:^|[^A-Za-z])stream(?![A-Za-z])/.test(raw);
}
function normalizePdfText(text) {
	return text.replace(/[ \t]+/g, " ").replace(/\n{3,}/g, "\n\n").trim();
}
function pageBodies(raw) {
	const ranges = streamRanges(raw);
	const markers = [];
	const re = /\/Type\s*\/Page(?!s)/g;
	let match;
	while ((match = re.exec(raw)) !== null) if (!indexInRanges(ranges, match.index)) markers.push(match.index);
	if (markers.length === 0) return [raw];
	return markers.map((marker, index) => {
		const objAt = raw.lastIndexOf("obj", marker);
		const start = objAt >= 0 && marker - objAt < 4e3 && !indexInRanges(ranges, objAt) && (index === 0 || objAt > markers[index - 1]) ? objAt : marker;
		return raw.slice(start, markers[index + 1] ?? raw.length);
	});
}
function pageText(body, budget) {
	const spans = [];
	let sawStream = false;
	let from = 0;
	while (from < body.length) {
		const at = findKeyword(body, "stream", from);
		if (at < 0) break;
		const dataAt = streamDataOffset(body, at);
		if (dataAt < 0) {
			from = at + 1;
			continue;
		}
		const end = findKeyword(body, "endstream", dataAt);
		if (end < 0) break;
		sawStream = true;
		const rawPayload = body.slice(dataAt, end);
		const trimmed = trimOneEol(rawPayload);
		const dict = dictionaryBefore(body, at);
		let decoded = trimmed;
		if (/\/FlateDecode\b/.test(dict) && !/\/Subtype\s*\/Image\b/.test(dict)) {
			const inflated = inflatePdfStream(trimmed) ?? (trimmed === rawPayload ? null : inflatePdfStream(rawPayload));
			if (inflated === "limit") throw new PdfLimitExceeded();
			if (typeof inflated !== "string") {
				from = end + 9;
				continue;
			}
			budget.used += inflated.length;
			if (budget.used > PDF_INFLATE_LIMIT) throw new PdfLimitExceeded();
			decoded = inflated;
		}
		const texts = collectSpans(decoded);
		if (texts.length) spans.push(texts.join(lineGap(decoded)));
		from = end + 9;
	}
	if (!sawStream) {
		const texts = collectSpans(body);
		if (texts.length) spans.push(texts.join(lineGap(body)));
	}
	return spans.join("\n").trim();
}
function dictionaryBefore(body, streamAt) {
	const window = body.slice(Math.max(0, streamAt - 16e3), streamAt);
	let cut = 0;
	for (const marker of [
		"endstream",
		"endobj",
		"obj"
	]) {
		const at = window.lastIndexOf(marker);
		if (at > cut) cut = at;
	}
	return window.slice(cut);
}
function streamRanges(raw) {
	const ranges = [];
	let from = 0;
	while (from < raw.length) {
		const at = findKeyword(raw, "stream", from);
		if (at < 0) break;
		const dataAt = streamDataOffset(raw, at);
		if (dataAt < 0) {
			from = at + 1;
			continue;
		}
		const end = findKeyword(raw, "endstream", dataAt);
		if (end < 0) break;
		ranges.push([dataAt, end]);
		from = end + 9;
	}
	return ranges;
}
function streamDataOffset(body, streamAt) {
	const dataAt = streamAt + 6;
	if (body.startsWith("\r\n", dataAt)) return dataAt + 2;
	if (body[dataAt] === "\n" || body[dataAt] === "\r") return dataAt + 1;
	return -1;
}
function trimOneEol(payload) {
	if (payload.endsWith("\r\n")) return payload.slice(0, -2);
	if (payload.endsWith("\n") || payload.endsWith("\r")) return payload.slice(0, -1);
	return payload;
}
function findKeyword(body, word, from) {
	let at = from;
	while (at < body.length) {
		const found = body.indexOf(word, at);
		if (found < 0) return -1;
		const prev = found === 0 ? " " : body[found - 1];
		const nextIndex = found + word.length;
		const next = body[nextIndex] ?? "";
		const followedByEndobj = word === "endstream" && body.startsWith("endobj", nextIndex);
		if (isDelimiter(prev) && (next === "" || isDelimiter(next) || followedByEndobj)) return found;
		at = found + 1;
	}
	return -1;
}
function isDelimiter(ch) {
	return ch === "" || /[^A-Za-z0-9]/.test(ch);
}
function indexInRanges(ranges, index) {
	return ranges.some(([start, end]) => index >= start && index < end);
}
function lineGap(body) {
	return /[-\d.]+\s+[-\d.]+\s+Td/.test(body) ? "\n" : " ";
}
function collectSpans(body) {
	const spans = [];
	let i = 0;
	while (i < body.length) {
		const ch = body[i];
		if (ch === "(") {
			const parsed = readLiteral(body, i);
			if (parsed.end <= i) {
				i++;
				continue;
			}
			i = parsed.end;
			if (showsText(body, i)) spans.push(parsed.text);
			continue;
		}
		if (ch === "<" && body[i + 1] !== "<") {
			const parsed = readHex(body, i);
			if (!parsed || parsed.end <= i) {
				i++;
				continue;
			}
			i = parsed.end;
			if (showsText(body, i)) spans.push(parsed.text);
			continue;
		}
		if (ch === "[") {
			const parsed = readTextArray(body, i);
			if (parsed && parsed.end > i) {
				if (parsed.text) spans.push(parsed.text);
				i = parsed.end;
				continue;
			}
		}
		i++;
	}
	return spans.filter((span) => span.length > 0);
}
function showsText(body, index) {
	const match = /^[ \t\r\n]*([A-Za-z'"]+)/.exec(body.slice(index));
	return match != null && (match[1] === "Tj" || match[1] === "'" || match[1] === "\"");
}
function readTextArray(body, start) {
	let i = start + 1;
	let text = "";
	while (i < body.length) {
		const ch = body[i];
		if (ch === "(") {
			const parsed = readLiteral(body, i);
			if (parsed.end <= i) return null;
			text += parsed.text;
			i = parsed.end;
			continue;
		}
		if (ch === "<" && body[i + 1] !== "<") {
			const parsed = readHex(body, i);
			if (parsed && parsed.end > i) {
				text += parsed.text;
				i = parsed.end;
				continue;
			}
		}
		if (ch === "]") {
			const match = /^[ \t\r\n]*(TJ)/.exec(body.slice(i + 1));
			if (!match) return null;
			return {
				text,
				end: i + 1 + match[0].length
			};
		}
		i++;
	}
	return null;
}
function readLiteral(body, start) {
	let i = start + 1;
	let depth = 1;
	let raw = "";
	while (i < body.length && depth > 0) {
		const ch = body[i];
		if (ch === "\\") {
			raw += ch;
			if (i + 1 < body.length) raw += body[++i];
			i++;
			continue;
		}
		if (ch === "(") depth++;
		if (ch === ")") {
			depth--;
			if (depth === 0) break;
		}
		raw += ch;
		i++;
	}
	return {
		text: unescapeLiteral(raw),
		end: Math.min(body.length, i + 1)
	};
}
function readHex(body, start) {
	if (body[start + 1] === "<") return null;
	let hex = "";
	let i = start + 1;
	while (i < body.length) {
		const ch = body[i];
		if (ch === ">") return {
			text: decodeHexString(hex),
			end: i + 1
		};
		if (/[0-9a-fA-F]/.test(ch)) hex += ch;
		else if (!/\s/.test(ch)) return null;
		i++;
	}
	return null;
}
function unescapeLiteral(body) {
	let out = "";
	for (let i = 0; i < body.length; i++) {
		const ch = body[i];
		if (ch !== "\\") {
			out += ch;
			continue;
		}
		const next = body[++i];
		if (next == null) break;
		if (next === "n") out += "\n";
		else if (next === "r") out += "\r";
		else if (next === "t") out += "	";
		else if (next === "b") out += "\b";
		else if (next === "f") out += "\f";
		else if (next === "(" || next === ")" || next === "\\") out += next;
		else if (next === "\n") continue;
		else if (next === "\r") {
			if (body[i + 1] === "\n") i++;
		} else if (next >= "0" && next <= "7") {
			let oct = next;
			for (let k = 0; k < 2; k++) {
				const digit = body[i + 1];
				if (digit >= "0" && digit <= "7") {
					oct += digit;
					i++;
				} else break;
			}
			out += String.fromCharCode(Number.parseInt(oct, 8) & 255);
		} else out += next;
	}
	return decodeUtf16Bytes(out);
}
function decodeUtf16Bytes(text) {
	if (text.length < 2 || text.charCodeAt(0) !== 254 || text.charCodeAt(1) !== 255) return text;
	let out = "";
	for (let i = 2; i + 1 < text.length; i += 2) {
		const code = text.charCodeAt(i) << 8 | text.charCodeAt(i + 1) & 255;
		if (code > 0) out += String.fromCharCode(code);
	}
	return out;
}
function decodeHexString(hex) {
	if (hex.length % 2 === 1) hex += "0";
	if (!hex) return "";
	const bytes = new Uint8Array(hex.length / 2);
	for (let i = 0; i < bytes.length; i++) {
		const value = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
		if (!Number.isFinite(value)) return "";
		bytes[i] = value;
	}
	if (bytes.length >= 2 && bytes[0] === 254 && bytes[1] === 255) return utf16be(bytes.subarray(2));
	if (bytes.length >= 2 && bytes.length % 2 === 0 && mostlyUtf16Be(bytes)) return utf16be(bytes);
	return bytesToLatin1(bytes);
}
function utf16be(bytes) {
	let out = "";
	for (let i = 0; i + 1 < bytes.length; i += 2) {
		const code = bytes[i] << 8 | bytes[i + 1];
		if (code > 0) out += String.fromCharCode(code);
	}
	return out;
}
function mostlyUtf16Be(bytes) {
	let zeros = 0;
	const pairs = bytes.length / 2;
	for (let i = 0; i < bytes.length; i += 2) if (bytes[i] === 0) zeros++;
	return zeros * 2 >= pairs;
}
function inflatePdfStream(latin1) {
	if (!latin1) return null;
	const bytes = latin1ToBytes(latin1);
	const zlibbed = tryInflate(bytes, "zlib");
	if (zlibbed === "limit") return "limit";
	if (zlibbed) return bytesToLatin1(zlibbed);
	const raw = tryInflate(bytes, "raw");
	if (raw === "limit") return "limit";
	if (raw) return bytesToLatin1(raw);
	return null;
}
function tryInflate(bytes, mode) {
	try {
		const out = /* @__PURE__ */ new Uint8Array(2000001);
		const inflated = mode === "zlib" ? unzlibSync(bytes, { out }) : inflateSync(bytes, { out });
		if (inflated.length > PDF_INFLATE_LIMIT) return "limit";
		return inflated;
	} catch {
		return null;
	}
}
function bytesToLatin1(bytes) {
	let out = "";
	const chunk = 8192;
	for (let i = 0; i < bytes.length; i += chunk) out += String.fromCharCode.apply(null, bytes.subarray(i, i + chunk));
	return out;
}
function latin1ToBytes(latin1) {
	const bytes = new Uint8Array(latin1.length);
	for (let i = 0; i < latin1.length; i++) bytes[i] = latin1.charCodeAt(i) & 255;
	return bytes;
}
//#endregion
//#region src/web/extraction/structured_content.ts
/** Parse public structured documents without treating their contents as HTML. */
function extractStructuredContent(body, mime) {
	const source = body.replace(/^\uFEFF/, "");
	if (source.length > 5242880) throw new Error("Structured document exceeds 5 MiB.");
	if (mime === "application/json" || mime.endsWith("+json")) {
		const value = JSON.parse(source);
		const text = JSON.stringify(value, null, 2);
		if (text.length > 10485760) throw new Error("Expanded JSON exceeds 10 MiB.");
		return {
			text,
			extractionMethod: "structured_json",
			rows: Array.isArray(value) ? value.length : void 0
		};
	}
	const delimiter = mime === "text/tab-separated-values" ? "	" : ",";
	const rows = [];
	let row = [], cell = "", quoted = false, afterQuote = false;
	function field() {
		row.push(cell);
		cell = "";
		afterQuote = false;
		if (row.length > 256) throw new Error("Delimited document exceeds 256 columns.");
	}
	function line() {
		field();
		rows.push(row);
		row = [];
		if (rows.length > 1e4) throw new Error("Delimited document exceeds 10000 rows. Request a smaller file.");
	}
	for (let index = 0; index < source.length; index++) {
		const char = source[index];
		if (quoted) {
			if (char === "\"" && source[index + 1] === "\"") {
				cell += "\"";
				index++;
			} else if (char === "\"") {
				quoted = false;
				afterQuote = true;
			} else cell += char;
		} else if (char === delimiter) field();
		else if (char === "\n" || char === "\r") {
			if (char === "\r" && source[index + 1] === "\n") index++;
			line();
		} else if (char === "\"" && !cell && !afterQuote) quoted = true;
		else if (afterQuote || char === "\"") throw new Error("Malformed delimited document: unexpected text around a quoted field.");
		else cell += char;
	}
	if (quoted) throw new Error("Malformed delimited document: unterminated quoted field.");
	if (cell || row.length || afterQuote) line();
	return {
		text: rows.map((cells, index) => `Row ${index + 1}: ${JSON.stringify(cells)}`).join("\n"),
		extractionMethod: delimiter === "	" ? "structured_tsv" : "structured_csv",
		rows: rows.length
	};
}
//#endregion
//#region src/web/evidence/source_registry.ts
var SourceManager = class {
	sources = /* @__PURE__ */ new Map();
	urlToId = /* @__PURE__ */ new Map();
	counter = 1;
	registerDocument(doc) {
		const existing = this.urlToId.get(doc.url);
		if (existing) return existing;
		const sourceId = `S${this.counter++}`;
		const meta = {
			id: sourceId,
			title: doc.title || "Untitled Document",
			url: doc.url,
			domain: doc.domain || extractDomain(doc.url),
			publishedAt: doc.publishedAt,
			author: doc.author,
			sourceType: doc.sourceType,
			authorityScore: doc.authorityScore
		};
		this.sources.set(sourceId, meta);
		this.urlToId.set(doc.url, sourceId);
		return sourceId;
	}
	getSource(id) {
		return this.sources.get(id);
	}
	getSourceIdForUrl(url) {
		return this.urlToId.get(url);
	}
	getAllSources() {
		const registry = {};
		for (const [k, v] of this.sources.entries()) registry[k] = v;
		return registry;
	}
	formatSourcesForContext() {
		const lines = [];
		for (const [id, meta] of this.sources.entries()) {
			const dateStr = meta.publishedAt ? ` (Published: ${meta.publishedAt})` : "";
			lines.push(`[${id}] "${meta.title}"${dateStr} - ${meta.url}`);
		}
		return lines.join("\n");
	}
};
//#endregion
//#region src/web/observability/trace.ts
var TraceCollector = class {
	trace;
	stageTimers = /* @__PURE__ */ new Map();
	constructor() {
		this.trace = {
			queriesGenerated: 0,
			searchResults: 0,
			uniqueResults: 0,
			pagesSelected: 0,
			pagesFetched: 0,
			fetchFailures: 0,
			extractedTokens: 0,
			chunksCreated: 0,
			chunksAfterRetrieval: 0,
			chunksAfterRerank: 0,
			evidenceClaims: 0,
			finalEvidenceTokens: 0,
			finalPromptTokens: 0,
			verifiedClaims: 0,
			unsupportedClaims: 0,
			conflictingClaims: 0,
			providerUsed: "unknown",
			fallbackUsed: false,
			fallbackReason: "",
			latencies: {},
			totalLatencyMs: 0
		};
	}
	startTimer(stage) {
		this.stageTimers.set(stage, Date.now());
	}
	endTimer(stage) {
		const start = this.stageTimers.get(stage);
		if (start) {
			this.trace.latencies[stage] = Date.now() - start;
			this.stageTimers.delete(stage);
		}
	}
	update(patch) {
		Object.assign(this.trace, patch);
	}
	getTrace() {
		return { ...this.trace };
	}
};
//#endregion
//#region src/web/observability/metrics.ts
function formatDiagnosticReport(session) {
	const t = session.trace;
	const compressionRatio = t.finalEvidenceTokens > 0 ? (t.extractedTokens / t.finalEvidenceTokens).toFixed(1) + "x" : "N/A";
	const verifiedFrac = t.evidenceClaims > 0 ? `${t.verifiedClaims}/${t.evidenceClaims}` : "0/0";
	const citationsCount = session.answer ? (session.answer.match(/\[S\d+\]/g) || []).length : 0;
	const queryFailures = (t.queryFailures || []).map((f) => `  - ${f.query}: ${f.error}`).join("\n");
	const diagnostics = t.searxngDiagnostics ? `\nEngine diagnostics: ${JSON.stringify(t.searxngDiagnostics)}` : "";
	const pagination = t.paginationPages?.length ? `\nPagination pages fetched: ${t.paginationPages.join(", ")}` : "";
	return `
--- RESEARCH DIAGNOSTICS REPORT ---
Question: ${session.question}
Routing: ${session.route.vertical} (${session.route.complexity || "medium"}, Freshness: ${session.route.freshness})
Provider: ${t.providerUsed}${t.fallbackUsed ? ` (fallback: ${t.fallbackReason})` : t.fallbackReason ? ` (${t.fallbackReason})` : ""}
Queries: ${t.queriesGenerated}${t.searchFailureCount ? ` (${t.searchFailureCount} failed)` : ""}
${session.queries.map((query, index) => `  ${index + 1}. ${query.query}`).join("\n")}${queryFailures ? `\nQuery failures:\n${queryFailures}` : ""}${diagnostics}${pagination}
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
//#endregion
//#region src/web/index.ts
var WebSearchEngine = class {
	config;
	searchProvider;
	fallbackProvider;
	searchStorage;
	fetcher;
	jsRender;
	domainStats;
	inflightFetches = /* @__PURE__ */ new Map();
	onProgress;
	constructor(deps = {}) {
		this.config = createConfig(deps.config);
		const google = this.config.googleApiKey && this.config.googleCxId ? new GoogleSearchProvider(this.config.googleApiKey, this.config.googleCxId) : void 0;
		const searxng = new SearXNGProvider(this.config.searxngBaseUrl, this.config.searxngTimeoutMs, this.config.searxngEngines, this.config.searxngDisabledEngines);
		this.searchProvider = deps.searchProvider || (this.config.searchProvider === "google" && google ? google : searxng);
		this.fallbackProvider = !deps.searchProvider && this.config.searchFallback?.enabled ? this.searchProvider === google ? searxng : google : void 0;
		this.searchStorage = deps.storage;
		this.fetcher = deps.fetcher || new HttpFetcher(this.config.fetch.globalConcurrency, this.config.fetch.perDomainConcurrency);
		this.jsRender = this.config.fetch.jsRenderFallback ? deps.jsRender : void 0;
		this.domainStats = this.config.fetch.domainLearning ? new DomainStatsStore(deps.storage ?? new InMemoryStorageAdapter()) : void 0;
		this.onProgress = deps.onProgress;
	}
	fetchCascade(url, options) {
		const pending = this.inflightFetches.get(url);
		if (pending) return pending;
		const promise = fetchPageCascade(url, this.fetcher, options).finally(() => {
			if (this.inflightFetches.get(url) === promise) this.inflightFetches.delete(url);
		});
		this.inflightFetches.set(url, promise);
		return promise;
	}
	async claimGoogleQuota(queryCount = 1) {
		const limit = this.config.searchFallback?.googleDailyLimit ?? 90;
		if (!this.searchStorage) return true;
		const key = `google_search_count_${(/* @__PURE__ */ new Date()).toISOString().split("T")[0]}`;
		const used = await this.searchStorage.get("quota", key) ?? 0;
		if (used >= limit) return false;
		await this.searchStorage.set("quota", key, used + queryCount, 9e4);
		return true;
	}
	async getHealthStatus() {
		return {
			...this.config.searchProvider === "google" && (!this.config.googleApiKey || !this.config.googleCxId) ? {
				search: "not_configured",
				searchError: "Configure a Google API key and Custom Search Engine ID."
			} : this.searchProvider instanceof SearXNGProvider ? await this.searchProvider.checkHealth() : { search: "ready" },
			database: this.config.cache.enabled ? "configured" : "disabled",
			embeddingModel: "disabled",
			reranker: "disabled",
			llm: "agent_owned",
			google: this.config.googleApiKey && this.config.googleCxId ? "configured" : "not_configured"
		};
	}
	/** Direct search for agent-controlled research; no nested planning/synthesis. */
	async searchQueries(queries, filters = {}) {
		if (!Array.isArray(queries) || queries.length < 1 || queries.length > 4 || queries.some((q) => typeof q !== "string" || !q.trim() || new TextEncoder().encode(q).length > 8e3)) throw new Error("search requires 1-4 nonempty queries, at most 8000 bytes each");
		const planned = [...new Set(queries.map((q) => q.trim()))].map((query) => ({
			query,
			purpose: "agent research",
			freshness: filters.freshness || "any",
			language: filters.language,
			category: filters.category
		}));
		const cacheKey = JSON.stringify([
			this.config.searchProvider,
			this.config.searxngBaseUrl,
			planned
		]);
		const cached = this.config.cache.enabled ? await this.searchStorage?.get("agent_search", cacheKey) : null;
		if (cached) return cached;
		if (this.searchProvider instanceof GoogleSearchProvider && !await this.claimGoogleQuota(planned.length)) throw new Error("Google daily quota exhausted");
		const options = {
			maxConcurrentQueries: this.config.maxConcurrentQueries,
			retries: this.config.searchRetries,
			retryDelayMs: this.config.searchRetryDelayMs
		};
		const result = await new SearchService(this.searchProvider, options).executeSearches(planned, 10);
		if (result.results.length || result.meta.answers.length || result.meta.infoboxes.length || !this.fallbackProvider) {
			if (this.config.cache.enabled && !result.failureCount) await this.searchStorage?.set("agent_search", cacheKey, result, ttlForFreshness(filters.freshness || "any", this.config.cache.searchTtlSeconds.default));
			return result;
		}
		if (this.fallbackProvider instanceof GoogleSearchProvider && !await this.claimGoogleQuota(planned.length)) return result;
		const fallback = await new SearchService(this.fallbackProvider, options).executeSearches(planned, 10);
		return {
			...fallback,
			failureCount: result.failureCount + fallback.failureCount,
			failures: [...result.failures, ...fallback.failures],
			meta: mergeSearchMeta(result.meta, fallback.meta)
		};
	}
	async fetchUrl(rawUrl) {
		const cache = this.config.cache.enabled ? new DocumentCache(this.searchStorage) : null;
		const cached = await cache?.get(rawUrl);
		if (cached) return cached;
		const document = await this.retrieveUrl(rawUrl);
		await cache?.set(document);
		return document;
	}
	async retrieveUrl(rawUrl) {
		const outcome = await this.fetchCascade(rawUrl, {
			timeoutSeconds: this.config.fetch.timeoutSeconds,
			maxBytes: this.config.fetch.maxBytes,
			userAgent: this.config.fetch.userAgent,
			waybackFallback: this.config.fetch.waybackFallback,
			domainStats: this.domainStats,
			jsRender: this.jsRender
		});
		const res = outcome.raw;
		const mime = (res?.mimeType || "").split(";")[0].trim().toLowerCase();
		if (res?.success && typeof res.body === "string" && ([
			"application/json",
			"text/csv",
			"application/csv",
			"text/tab-separated-values"
		].includes(mime) || mime.endsWith("+json"))) {
			const structured = extractStructuredContent(res.body, mime);
			const url = res.finalUrl || rawUrl;
			return {
				id: `data-${crypto.randomUUID()}`,
				url,
				domain: new URL(url).hostname,
				title: new URL(url).pathname.split("/").pop() || "Structured document",
				text: structured.text,
				retrievedAt: (/* @__PURE__ */ new Date()).toISOString(),
				searchResultIds: [],
				metadata: {
					extractionMethod: structured.extractionMethod,
					rowCount: structured.rows
				}
			};
		}
		if (res?.success && res.body && (mime === "application/pdf" || /\.pdf(\?|#|$)/i.test(res.finalUrl || rawUrl))) {
			const latin1 = res.body || "";
			const pdf = extractPdfText(Uint8Array.from(latin1, (ch) => ch.charCodeAt(0) & 255));
			switch (pdf.status) {
				case "text_extracted": break;
				case "no_extractable_text": throw new Error("This PDF has no extractable text.");
				case "password_required": throw new Error("This PDF is password-protected and cannot be extracted locally.");
				case "invalid_document": throw new Error("The response is not a valid PDF.");
				case "limit_exceeded": throw new Error("PDF exceeds the local extraction size limit.");
				default: {
					const unexpected = pdf.status;
					throw new Error(`PDF extraction failed (${String(unexpected)}).`);
				}
			}
			return {
				id: `pdf-${crypto.randomUUID()}`,
				url: res.finalUrl || rawUrl,
				domain: new URL(res.finalUrl || rawUrl).hostname,
				title: rawUrl.split("/").pop() || "PDF document",
				text: pdf.text,
				pages: pdf.pages,
				retrievedAt: (/* @__PURE__ */ new Date()).toISOString(),
				searchResultIds: [],
				metadata: {
					extractionMethod: "pdf_text",
					pageCount: pdf.pages.length
				}
			};
		}
		if (outcome.document) return {
			id: `fetch-${crypto.randomUUID()}`,
			url: outcome.document.finalUrl || rawUrl,
			domain: new URL(outcome.document.finalUrl || rawUrl).hostname,
			title: outcome.document.title || rawUrl,
			text: outcome.document.text,
			headings: outcome.document.headings,
			links: outcome.document.links,
			author: outcome.document.author,
			publishedAt: outcome.document.publishedAt,
			retrievedAt: (/* @__PURE__ */ new Date()).toISOString(),
			searchResultIds: [],
			metadata: {
				extractionMethod: outcome.method === "github_raw" ? "github_raw" : outcome.method === "js_render" ? "js_render" : outcome.method === "wayback" ? "wayback" : "main_content",
				archivedAt: outcome.document.archivedAt
			}
		};
		throw new Error(`Fetch failed: ${outcome.error || res?.error || "unknown error"}`);
	}
	async research(question, options = {}) {
		if (!this.config.enabled) throw new Error("Web research is disabled");
		if (!question.trim() || new TextEncoder().encode(question).length > 8e3) throw new Error("Question must contain 1–8000 bytes");
		const startedAt = (/* @__PURE__ */ new Date()).toISOString();
		this.onProgress?.({
			stage: "search",
			message: "Searching public sources"
		});
		const outcome = await this.searchQueries([question], options);
		const sources = new SourceManager();
		const documents = outcome.results.map((hit, index) => ({
			id: `search-${index + 1}`,
			url: hit.url,
			title: hit.title,
			domain: new URL(hit.url).hostname,
			text: hit.snippet || "",
			retrievedAt: (/* @__PURE__ */ new Date()).toISOString(),
			searchResultIds: [hit.id],
			metadata: { extractionMethod: "snippet_only" }
		}));
		const snippets = documents.map((doc) => `[${sources.registerDocument(doc)}] ${doc.title} — ${doc.url}\n${doc.text}`);
		const trace = new TraceCollector();
		trace.update({
			queriesGenerated: 1,
			searchResults: outcome.results.length,
			uniqueResults: documents.length,
			searchFailureCount: outcome.failureCount,
			queryFailures: outcome.failures,
			providerUsed: this.config.searchProvider,
			searxngDiagnostics: this.searchProvider instanceof SearXNGProvider ? this.searchProvider.lastDiagnostics ? { ...this.searchProvider.lastDiagnostics } : void 0 : void 0,
			totalLatencyMs: Date.now() - Date.parse(startedAt)
		});
		return {
			id: `sess-${crypto.randomUUID()}`,
			question,
			normalizedQuery: question.trim(),
			route: {
				vertical: "GENERAL_WEB",
				confidence: 1,
				requiresExternalData: true,
				requiresWebSearch: true,
				freshness: options.freshness || "any"
			},
			queries: [{
				query: question,
				purpose: "agent search",
				freshness: options.freshness || "any"
			}],
			results: outcome.results,
			searchMeta: outcome.meta,
			documents,
			chunks: [],
			retrievedChunks: [],
			rerankedChunks: [],
			evidence: [],
			sources: sources.getAllSources(),
			answer: snippets.join("\n\n"),
			tokenUsage: {
				systemBudget: 0,
				questionBudget: 0,
				evidenceBudget: 0,
				safetyMargin: 0,
				totalInputBudget: 0,
				estimatedEvidenceTokens: 0,
				finalPromptTokens: 0
			},
			startedAt,
			completedAt: (/* @__PURE__ */ new Date()).toISOString(),
			trace: trace.getTrace(),
			limitations: ["Search snippets are unverified source content. Open relevant pages before making factual claims."]
		};
	}
};
//#endregion
//#region scripts/web-storage.mjs
/** Dedicated research database: never expose arbitrary SQL to a model. */
var SqliteStorage = class {
	constructor(path) {
		this.db = new DatabaseSync(path);
		this.db.exec(`PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;
      CREATE TABLE IF NOT EXISTS cache (
        namespace TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
        expires_at INTEGER NOT NULL, byte_size INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(namespace,key));
      CREATE TABLE IF NOT EXISTS research (id TEXT PRIMARY KEY, session TEXT NOT NULL, created_at INTEGER NOT NULL);`);
		this.db.exec("BEGIN IMMEDIATE");
		try {
			if (!this.db.prepare("PRAGMA table_info(cache)").all().some((column) => column.name === "byte_size")) this.db.exec("ALTER TABLE cache ADD COLUMN byte_size INTEGER NOT NULL DEFAULT 0; UPDATE cache SET byte_size=length(CAST(value AS BLOB))");
			this.db.exec("COMMIT");
		} catch (error) {
			this.db.exec("ROLLBACK");
			throw error;
		}
		this.db.prepare("DELETE FROM cache WHERE expires_at <= ?").run(Date.now());
	}
	async get(table, key) {
		const row = this.db.prepare("SELECT value FROM cache WHERE namespace=? AND key=? AND expires_at>?").get(table, key, Date.now());
		return row ? JSON.parse(row.value) : null;
	}
	async set(table, key, value, ttl = 86400) {
		const serialized = JSON.stringify(value);
		if (Buffer.byteLength(serialized) > 10485760) return;
		this.db.exec("BEGIN IMMEDIATE");
		try {
			this.db.prepare("INSERT OR REPLACE INTO cache(namespace,key,value,expires_at,byte_size) VALUES(?,?,?,?,?)").run(table, key, serialized, Date.now() + ttl * 1e3, Buffer.byteLength(serialized));
			this.db.prepare("DELETE FROM cache WHERE expires_at <= ?").run(Date.now());
			if (table.startsWith("weather-")) this.db.prepare("DELETE FROM cache WHERE namespace=? AND rowid NOT IN (SELECT rowid FROM cache WHERE namespace=? ORDER BY expires_at DESC,rowid DESC LIMIT 64)").run(table, table);
			this.db.exec("DELETE FROM cache WHERE rowid NOT IN (SELECT rowid FROM cache ORDER BY expires_at DESC,rowid DESC LIMIT 2000)");
			let size = this.db.prepare("SELECT COALESCE(SUM(byte_size),0) AS bytes FROM cache").get().bytes;
			while (size > 67108864) {
				const oldest = this.db.prepare("SELECT rowid,byte_size AS bytes FROM cache ORDER BY expires_at,rowid LIMIT 1").get();
				if (!oldest) break;
				this.db.prepare("DELETE FROM cache WHERE rowid=?").run(oldest.rowid);
				size -= oldest.bytes;
			}
			this.db.exec("COMMIT");
		} catch (error) {
			this.db.exec("ROLLBACK");
			throw error;
		}
	}
	async delete(table, key) {
		this.db.prepare("DELETE FROM cache WHERE namespace=? AND key=?").run(table, key);
	}
	async clear(table) {
		if (table) this.db.prepare("DELETE FROM cache WHERE namespace=?").run(table);
		else this.db.exec("DELETE FROM cache");
	}
	save(session) {
		this.db.prepare("INSERT OR REPLACE INTO research VALUES(?,?,?)").run(session.id, JSON.stringify(session), Date.now());
	}
	load(id) {
		const row = this.db.prepare("SELECT session FROM research WHERE id=?").get(id);
		if (!row) throw new Error("Unknown research session");
		return JSON.parse(row.session);
	}
	close() {
		this.db.close();
	}
};
//#endregion
//#region src/web/fetch/robots_policy.ts
/** RFC 9309 group selection and longest matching path, including * and $. */
var RobotsChecker = class {
	parseRobotsTxt(text) {
		const rules = [];
		let agents = [], allow = [], disallow = [], directives = false;
		const flush = () => {
			for (const userAgent of agents) rules.push({
				userAgent,
				allow: [...allow],
				disallow: [...disallow]
			});
			agents = [];
			allow = [];
			disallow = [];
			directives = false;
		};
		for (const raw of text.split(/\r?\n/)) {
			const line = raw.split("#")[0].trim();
			const colon = line.indexOf(":");
			if (colon < 0) continue;
			const key = line.slice(0, colon).trim().toLowerCase(), value = line.slice(colon + 1).trim();
			if (key === "user-agent") {
				if (directives) flush();
				agents.push(value.toLowerCase());
			} else if (agents.length && (key === "allow" || key === "disallow")) {
				directives = true;
				if (value) (key === "allow" ? allow : disallow).push(value);
			}
		}
		flush();
		return rules;
	}
	isPathAllowed(path, rules, userAgent = "*") {
		const explicit = rules.filter((r) => r.userAgent !== "*" && userAgent.toLowerCase().includes(r.userAgent));
		const longest = Math.max(0, ...explicit.map((r) => r.userAgent.length));
		const selected = explicit.length ? explicit.filter((r) => r.userAgent.length === longest) : rules.filter((r) => r.userAgent === "*");
		let bestLength = -1, allowed = true;
		for (const rule of selected) for (const [patterns, permit] of [[rule.disallow, false], [rule.allow, true]]) for (const pattern of patterns) {
			const end = pattern.endsWith("$");
			const body = end ? pattern.slice(0, -1) : pattern;
			const regex = "^" + body.split("*").map((p) => p.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join(".*") + (end ? "$" : "");
			if (new RegExp(regex).test(path) && (body.length > bestLength || body.length === bestLength && permit)) {
				bestLength = body.length;
				allowed = permit;
			}
		}
		return allowed;
	}
};
//#endregion
//#region src/web/fetch/rate_limiter.ts
/** Bounded global/per-domain semaphore. Wake all waiters to avoid cross-queue starvation. */
var BoundedConcurrencyLimiter = class {
	globalActive = 0;
	domainActive = /* @__PURE__ */ new Map();
	waiters = [];
	maxGlobal;
	maxPerDomain;
	constructor(maxGlobal = 8, maxPerDomain = 2) {
		if (!Number.isInteger(maxGlobal) || !Number.isInteger(maxPerDomain) || maxGlobal < 1 || maxPerDomain < 1) throw new Error("Concurrency must be positive integers");
		this.maxGlobal = maxGlobal;
		this.maxPerDomain = maxPerDomain;
	}
	async acquire(domain) {
		while (this.globalActive >= this.maxGlobal || (this.domainActive.get(domain) || 0) >= this.maxPerDomain) await new Promise((resolve) => this.waiters.push(resolve));
		this.globalActive++;
		this.domainActive.set(domain, (this.domainActive.get(domain) || 0) + 1);
		let released = false;
		return () => {
			if (released) return;
			released = true;
			this.globalActive--;
			const active = (this.domainActive.get(domain) || 1) - 1;
			if (active) this.domainActive.set(domain, active);
			else this.domainActive.delete(domain);
			this.waiters.splice(0).forEach((resolve) => resolve());
		};
	}
};
//#endregion
//#region scripts/web-transport.mjs
var denied = new BlockList();
for (const [address, prefix] of [
	["0.0.0.0", 8],
	["10.0.0.0", 8],
	["100.64.0.0", 10],
	["127.0.0.0", 8],
	["169.254.0.0", 16],
	["172.16.0.0", 12],
	["192.0.0.0", 24],
	["192.0.2.0", 24],
	["192.168.0.0", 16],
	["198.18.0.0", 15],
	["198.51.100.0", 24],
	["203.0.113.0", 24],
	["224.0.0.0", 3]
]) denied.addSubnet(address, prefix, "ipv4");
var globalV6 = new BlockList();
globalV6.addSubnet("2000::", 3, "ipv6");
denied.addSubnet("2001::", 23, "ipv6");
denied.addSubnet("2001:db8::", 32, "ipv6");
denied.addSubnet("2002::", 16, "ipv6");
function isPublicAddress(ip) {
	const family = isIP(ip);
	return family === 4 ? !denied.check(ip, "ipv4") : family === 6 && globalV6.check(ip, "ipv6") && !denied.check(ip, "ipv6");
}
var USER_AGENTS = [
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
	"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
	"Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:126.0) Gecko/20100101 Firefox/126.0",
	"Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Safari/605.1.15",
	"Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36"
];
async function addressesFor(url) {
	if (!["http:", "https:"].includes(url.protocol) || url.username || url.password) throw new Error("Only credential-free HTTP(S) URLs are allowed");
	const host = url.hostname.replace(/^\[|\]$/g, "").replace(/\.$/, "");
	if (/^(localhost|metadata|instance-data)$|\.(local|internal|localhost|corp|home|lan)$/i.test(host)) throw new Error("Internal host blocked");
	let timer;
	try {
		const addresses = isIP(host) ? [{
			address: host,
			family: isIP(host)
		}] : await Promise.race([lookup(host, { all: true }), new Promise((_, reject) => {
			timer = setTimeout(() => reject(/* @__PURE__ */ new Error("DNS timeout")), 4e3);
		})]);
		if (!addresses.length || addresses.some((a) => !isPublicAddress(a.address))) throw new Error("Non-public address blocked");
		return addresses;
	} finally {
		clearTimeout(timer);
	}
}
/** DNS is resolved once, checked, then pinned into the actual socket lookup. */
async function pinnedGet(url, options) {
	const addresses = await addressesFor(url);
	return new Promise((resolve, reject) => {
		const request = (url.protocol === "https:" ? https : http).get(url, {
			agent: false,
			lookup: (_host, opts, callback) => opts.all ? callback(null, addresses) : callback(null, addresses[0].address, addresses[0].family),
			headers: {
				"User-Agent": options.userAgent || USER_AGENTS[Math.floor(Math.random() * USER_AGENTS.length)],
				"Accept": "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
				"Accept-Language": "en-US,en;q=0.9",
				"Accept-Encoding": "identity",
				"Sec-Fetch-Dest": "document",
				"Sec-Fetch-Mode": "navigate",
				"Sec-Fetch-Site": "none",
				"Sec-Fetch-User": "?1",
				"Upgrade-Insecure-Requests": "1"
			}
		}, (response) => {
			if (Number(response.headers["content-length"]) > options.maxBytes) {
				request.destroy(/* @__PURE__ */ new Error("Response exceeds byte limit"));
				return;
			}
			let size = 0;
			const parts = [];
			const encoding = response.headers["content-encoding"];
			const decoder = encoding === "gzip" ? createGunzip() : encoding === "br" ? createBrotliDecompress() : encoding === "deflate" ? createInflate() : null;
			const stream = decoder ? response.pipe(decoder) : response;
			stream.on("data", (chunk) => {
				size += chunk.length;
				if (size > options.maxBytes) request.destroy(/* @__PURE__ */ new Error("Response exceeds byte limit"));
				else parts.push(chunk);
			});
			response.on("error", reject);
			stream.on("error", reject);
			stream.on("end", () => resolve({
				status: response.statusCode,
				headers: response.headers,
				body: Buffer.concat(parts)
			}));
		});
		const timer = setTimeout(() => request.destroy(/* @__PURE__ */ new Error("Fetch deadline exceeded")), options.timeoutSeconds * 1e3);
		request.on("error", reject);
		request.on("close", () => clearTimeout(timer));
	});
}
var PinnedPageFetcher = class {
	constructor(globalConcurrency = 8, perDomainConcurrency = 2, get = pinnedGet) {
		this.get = get;
		this.limiter = new BoundedConcurrencyLimiter(globalConcurrency, perDomainConcurrency);
		this.robots = /* @__PURE__ */ new Map();
		this.queues = /* @__PURE__ */ new Map();
		this.cooldowns = /* @__PURE__ */ new Map();
	}
	async limited(url, options) {
		const domain = url.hostname;
		const task = (this.queues.get(domain) || Promise.resolve()).catch(() => {}).then(async () => {
			if ((this.cooldowns.get(domain) || 0) > Date.now()) throw new Error("Host Retry-After cooldown active");
			let lastError;
			for (let attempt = 0; attempt < 3; attempt++) try {
				await new Promise((resolve) => setTimeout(resolve, 1e3));
				const release = await this.limiter.acquire(domain);
				let response;
				try {
					response = await this.get(url, options);
				} finally {
					release();
				}
				if ([429, 503].includes(response.status)) {
					const value = response.headers["retry-after"];
					const deadline = /^\d+$/.test(value || "") ? Date.now() + Number(value) * 1e3 : Date.parse(value);
					this.cooldowns.set(domain, Math.max(Date.now() + 3e4, Number.isFinite(deadline) ? deadline : 0));
					lastError = /* @__PURE__ */ new Error(`HTTP ${response.status}`);
					if (attempt < 2) {
						await new Promise((r) => setTimeout(r, 3e3));
						continue;
					}
					throw lastError;
				}
				return response;
			} catch (err) {
				lastError = err;
				if (err.message === "Host Retry-After cooldown active") throw err;
				if (attempt < 2) {
					await new Promise((r) => setTimeout(r, 3e3));
					continue;
				}
				throw err;
			}
			throw lastError;
		});
		this.queues.set(domain, task);
		return task;
	}
	async allowed(url, options) {
		if (!this.robots.has(url.origin)) {
			const task = (async () => {
				const res = await this.limited(new URL("/robots.txt", url), {
					...options,
					maxBytes: 512e3
				});
				if (res.status === 404 || res.status === 410) return [];
				if (res.status !== 200) return [];
				return new RobotsChecker().parseRobotsTxt(res.body.toString("utf8"));
			})();
			this.robots.set(url.origin, task);
		}
		return new RobotsChecker().isPathAllowed(url.pathname + url.search, await this.robots.get(url.origin), "LocalLM-Research");
	}
	async fetch(raw, overrides = {}) {
		const options = {
			timeoutSeconds: 10,
			maxBytes: 5242880,
			maxRedirects: 5,
			...overrides
		};
		const started = Date.now();
		let url;
		try {
			url = new URL(raw);
			for (let hop = 0; hop <= options.maxRedirects; hop++) {
				await addressesFor(url);
				if (!await this.allowed(url, options)) throw new Error("robots.txt disallows this page");
				const res = await this.limited(url, options);
				if ([
					301,
					302,
					303,
					307,
					308
				].includes(res.status)) {
					if (!res.headers.location) throw new Error("Redirect missing Location");
					url = new URL(res.headers.location, url);
					continue;
				}
				if (res.status !== 200) throw new Error(`Page unavailable: HTTP ${res.status}`);
				const mime = String(res.headers["content-type"] || "").split(";")[0].trim().toLowerCase();
				if (![
					"text/html",
					"text/plain",
					"text/markdown",
					"text/csv",
					"application/csv",
					"text/tab-separated-values",
					"application/xhtml+xml",
					"application/json",
					"application/pdf"
				].includes(mime) && !/^application\/[\w.+-]+\+json$/.test(mime)) throw new Error(`Unsupported MIME ${mime}`);
				const body = mime === "application/pdf" ? res.body.toString("latin1") : res.body.toString("utf8");
				return {
					url: raw,
					finalUrl: url.href,
					success: true,
					status: res.status,
					body,
					mimeType: mime,
					durationMs: Date.now() - started
				};
			}
			throw new Error("Redirect limit exceeded");
		} catch (error) {
			return {
				url: raw,
				finalUrl: url?.href || raw,
				success: false,
				error: String(error.message),
				durationMs: Date.now() - started
			};
		}
	}
};
//#endregion
//#region scripts/web-render.mjs
/**
* Last-resort render for pages a static fetch cannot read.
*
* The document and every later hop are retrieved with the pinned HTTP transport.
* Headless Edge, when it launches, gets an isolated non-persistent context and
* the bytes we already fetched. It is not navigated at the target URL.
*
* Routing is application policy, not an OS sandbox. Anything the route handler
* does not explicitly fulfill is aborted. If Edge cannot be launched, the
* result is null — there is no unrestricted browser fallback.
*/
var EDGE_CANDIDATES = [
	process.env.LOCALLM_EDGE_PATH,
	"C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
	"C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
	process.env.LOCALAPPDATA ? join(process.env.LOCALAPPDATA, "Microsoft\\Edge\\Application\\msedge.exe") : void 0
].filter(Boolean);
var REDIRECT_STATUSES = /* @__PURE__ */ new Set([
	301,
	302,
	303,
	307,
	308
]);
var HOP_HEADERS = /* @__PURE__ */ new Set([
	"connection",
	"keep-alive",
	"proxy-connection",
	"transfer-encoding",
	"upgrade",
	"content-encoding",
	"content-length",
	"content-disposition",
	"location",
	"set-cookie",
	"set-cookie2",
	"proxy-authenticate",
	"proxy-authorization"
]);
/**
* Isolation flags for browser.newContext. Not a persistent profile and not an
* OS sandbox: a dead proxy makes a request that escapes routing fail closed.
*/
var ISOLATED_CONTEXT_OPTIONS = Object.freeze({
	serviceWorkers: "block",
	acceptDownloads: false,
	proxy: { server: "http://127.0.0.1:1" }
});
function findEdgeBinary(candidates = EDGE_CANDIDATES, exists = existsSync) {
	return candidates.find((path) => exists(path)) || null;
}
function stripHost(host) {
	return String(host || "").trim().toLowerCase().replace(/^\[|\]$/g, "").replace(/\.$/, "");
}
function rawHostname(input) {
	const scheme = String(input || "").trim().match(/^[a-z][a-z0-9+.-]*:(.*)$/i);
	if (!scheme) return "";
	let rest = scheme[1];
	if (!rest.startsWith("//")) return "";
	rest = rest.slice(2);
	const cut = rest.search(/[/?#]/);
	const authority = cut === -1 ? rest : rest.slice(0, cut);
	const at = authority.lastIndexOf("@");
	const hostport = at === -1 ? authority : authority.slice(at + 1);
	if (hostport.startsWith("[")) {
		const end = hostport.indexOf("]");
		return end === -1 ? hostport : hostport.slice(1, end);
	}
	const colon = hostport.lastIndexOf(":");
	if (colon !== -1 && hostport.indexOf(":") === colon) return hostport.slice(0, colon);
	return hostport;
}
function isIpv4Mapped(host) {
	const clean = stripHost(host);
	return clean.startsWith("::ffff:") || /^(?:0+:){5}ffff:/i.test(clean);
}
function isInternalHost(host) {
	return /^(localhost|metadata|instance-data)$|\.(local|internal|localhost|corp|home|lan)$/i.test(host);
}
/** Canonical dotted IPv4 only: four decimal octets, no leading zeros. */
function isCanonicalDottedIpv4(host) {
	return /^(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(?:\.(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/.test(host);
}
function isDottedOrNumericIp(host) {
	return /^(?:\d+|0x[0-9a-f]+)(?:\.(?:\d+|0x[0-9a-f]+)){0,3}$/i.test(host);
}
/**
* Reject decimal, hex, octal, short, and IPv4-mapped spellings before any
* socket exists. Canonical dotted IPv4 is left for the public-address check.
*/
function alternateIpReason(rawUrl) {
	let parsed;
	try {
		parsed = new URL(rawUrl);
	} catch {
		return "invalid url";
	}
	if (parsed.username || parsed.password) return "credentials blocked";
	if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return "protocol blocked";
	const raw = stripHost(rawHostname(rawUrl));
	const canonical = stripHost(parsed.hostname);
	if (!canonical) return "empty host";
	if (isIpv4Mapped(raw) || isIpv4Mapped(canonical)) return "IPv4-mapped IPv6 blocked";
	if (raw && isDottedOrNumericIp(raw) && !isCanonicalDottedIpv4(raw)) return "alternate IP form blocked";
	if (raw && isIP(canonical) && raw !== canonical) return "alternate IP form blocked";
	return null;
}
/** Static block. Null means the URL may proceed to a fresh lookup. No DNS. */
function gateUrl(rawUrl) {
	const reason = alternateIpReason(rawUrl);
	if (reason) return reason;
	let parsed;
	try {
		parsed = new URL(rawUrl);
	} catch {
		return "invalid url";
	}
	const host = stripHost(parsed.hostname);
	if (isInternalHost(host)) return "internal host blocked";
	if (isIP(host) && !isPublicAddress(host)) return "Non-public address blocked";
	return null;
}
function headerValue(headers, name) {
	if (!headers) return void 0;
	const wanted = name.toLowerCase();
	for (const [key, value] of Object.entries(headers)) if (key.toLowerCase() === wanted) return Array.isArray(value) ? value[0] : value;
}
function asBuffer(body) {
	if (body == null) return Buffer.alloc(0);
	if (Buffer.isBuffer(body)) return body;
	if (typeof body === "string") return Buffer.from(body);
	if (body instanceof Uint8Array) return Buffer.from(body);
	return Buffer.from(String(body));
}
function addressBlockReason(address) {
	const clean = stripHost(address);
	if (isIpv4Mapped(clean)) return "IPv4-mapped IPv6 blocked";
	if (!isIP(clean) || !isPublicAddress(clean)) return "Non-public address blocked";
	return null;
}
async function resolvePublic(hostname, lookup, deadlineMs) {
	let timer;
	const pending = Promise.resolve().then(() => lookup(hostname));
	pending.catch(() => {});
	try {
		const addresses = await Promise.race([pending, new Promise((_, reject) => {
			timer = setTimeout(() => reject(/* @__PURE__ */ new Error("DNS timeout")), Math.min(4e3, Math.max(1, deadlineMs)));
		})]);
		if (!Array.isArray(addresses) || addresses.length === 0) return "DNS failed";
		for (const entry of addresses) {
			const reason = addressBlockReason(typeof entry === "string" ? entry : entry?.address);
			if (reason) return reason;
		}
		return null;
	} catch {
		return "DNS failed";
	} finally {
		clearTimeout(timer);
	}
}
function browserCapabilityReason(request) {
	const type = String(request?.resourceType || "").toLowerCase();
	if (request?.isWebSocket || type === "websocket") return "websocket blocked";
	if (request?.isDownload || type === "download") return "download blocked";
	if (request?.isPopup) return "popup blocked";
	if (request?.isServiceWorker || type === "serviceworker" || type === "service_worker") return "service worker blocked";
	return null;
}
function htmlMime(headers) {
	const mime = String(headerValue(headers, "content-type") || "").split(";")[0].trim().toLowerCase();
	if (!mime) return "text/html";
	if (mime === "text/html" || mime === "application/xhtml+xml") return mime;
	return null;
}
function sanitizeHeaders(headers) {
	const out = {};
	for (const [key, value] of Object.entries(headers || {})) {
		if (HOP_HEADERS.has(key.toLowerCase())) continue;
		if (value == null) continue;
		out[key] = Array.isArray(value) ? value.join(", ") : String(value);
	}
	return out;
}
function escapeAttr(value) {
	return String(value).replace(/[&"'<>]/g, (ch) => `&#${ch.charCodeAt(0)};`);
}
function withBase(html, finalUrl) {
	const tag = `<base href="${escapeAttr(finalUrl)}">`;
	if (/<head[^>]*>/i.test(html)) return html.replace(/<head[^>]*>/i, (match) => `${match}${tag}`);
	if (/<html[^>]*>/i.test(html)) return html.replace(/<html[^>]*>/i, (match) => `${match}<head>${tag}</head>`);
	return `<!doctype html><html><head>${tag}</head><body>${html}</body></html>`;
}
/**
* One render's HTTP policy. Lookup results are not cached: every hop calls
* `lookup` again, so a name that flips to a private address is blocked.
*/
function createMediationSession({ get = pinnedGet, lookup = defaultLookup, maxBytes = 5242880, timeoutMs = 2e4, maxRedirects = 5, maxRequests = 32, maxConcurrency = 4, now = Date.now } = {}) {
	if (!Number.isFinite(maxBytes) || maxBytes < 1) throw new Error("maxBytes must be positive");
	if (!Number.isFinite(timeoutMs) || timeoutMs < 1) throw new Error("timeoutMs must be positive");
	if (!Number.isInteger(maxRedirects) || maxRedirects < 0) throw new Error("maxRedirects must be a non-negative integer");
	if (!Number.isInteger(maxRequests) || maxRequests < 1) throw new Error("maxRequests must be a positive integer");
	if (!Number.isInteger(maxConcurrency) || maxConcurrency < 1) throw new Error("maxConcurrency must be a positive integer");
	const started = now();
	let bytes = 0;
	let requests = 0;
	let inFlight = 0;
	function remainingMs() {
		return timeoutMs - (now() - started);
	}
	function reserve() {
		if (remainingMs() <= 0) return "time limit";
		if (requests >= maxRequests) return "request limit";
		if (inFlight >= maxConcurrency) return "concurrency limit";
		requests += 1;
		inFlight += 1;
		return null;
	}
	async function classify(target) {
		const textual = typeof target === "string" ? target : target?.href;
		if (!textual) return {
			allow: false,
			reason: "invalid url"
		};
		const literal = gateUrl(textual);
		if (literal) return {
			allow: false,
			reason: literal
		};
		let parsed;
		try {
			parsed = new URL(textual);
		} catch {
			return {
				allow: false,
				reason: "invalid url"
			};
		}
		const host = stripHost(parsed.hostname);
		if (!isIP(host)) {
			const reason = await resolvePublic(host, lookup, remainingMs());
			if (reason) return {
				allow: false,
				reason,
				url: parsed
			};
		}
		return {
			allow: true,
			url: parsed
		};
	}
	async function exchange(rawUrl) {
		let current = String(rawUrl);
		for (let hop = 0; hop <= maxRedirects; hop++) {
			const limited = reserve();
			if (limited) return {
				ok: false,
				error: limited,
				finalUrl: safeHref(current)
			};
			let redirectTo = null;
			try {
				const decision = await classify(current);
				if (!decision.allow) return {
					ok: false,
					error: decision.reason,
					finalUrl: decision.url?.href || safeHref(current)
				};
				const parsed = decision.url;
				let response;
				try {
					response = await get(parsed, {
						maxBytes: Math.max(1, maxBytes - bytes),
						timeoutSeconds: Math.max(1, Math.ceil(remainingMs() / 1e3))
					});
				} catch (error) {
					return {
						ok: false,
						error: error?.message || "fetch failed",
						finalUrl: parsed.href
					};
				}
				if (remainingMs() <= 0) return {
					ok: false,
					error: "time limit",
					finalUrl: parsed.href
				};
				const body = asBuffer(response?.body);
				bytes += body.length;
				if (bytes > maxBytes) return {
					ok: false,
					error: "byte limit",
					finalUrl: parsed.href
				};
				if (REDIRECT_STATUSES.has(Number(response?.status))) {
					const location = headerValue(response.headers, "location");
					if (!location || !String(location).trim()) return {
						ok: false,
						error: "Redirect missing Location",
						finalUrl: parsed.href
					};
					const next = nextHop(String(location).trim(), parsed);
					if (next.error) return {
						ok: false,
						error: next.error,
						finalUrl: parsed.href
					};
					redirectTo = next.href;
				} else return {
					ok: true,
					finalUrl: parsed.href,
					origin: parsed.origin,
					status: Number(response?.status) || 0,
					headers: response?.headers || {},
					body
				};
			} finally {
				inFlight -= 1;
			}
			current = redirectTo;
		}
		return {
			ok: false,
			error: "Redirect limit exceeded",
			finalUrl: safeHref(current)
		};
	}
	async function mediate(request) {
		const capability = browserCapabilityReason(request);
		if (capability) return {
			action: "abort",
			reason: capability
		};
		const outcome = await exchange(request.url);
		if (!outcome.ok) return {
			action: "abort",
			reason: outcome.error,
			finalUrl: outcome.finalUrl
		};
		if (/attachment/i.test(String(headerValue(outcome.headers, "content-disposition") || ""))) return {
			action: "abort",
			reason: "download blocked",
			finalUrl: outcome.finalUrl
		};
		const mime = htmlMime(outcome.headers) || String(headerValue(outcome.headers, "content-type") || "application/octet-stream").split(";")[0].trim();
		return {
			action: "fulfill",
			reason: "mediated",
			finalUrl: outcome.finalUrl,
			origin: outcome.origin,
			fulfillment: {
				status: outcome.status || 200,
				headers: sanitizeHeaders(outcome.headers),
				body: outcome.body,
				contentType: mime || "application/octet-stream"
			}
		};
	}
	async function handleRoute(route, mainPage) {
		const described = describeRequest(route.request(), mainPage);
		let decision;
		try {
			decision = await mediate(described);
		} catch (error) {
			decision = {
				action: "abort",
				reason: error?.message || "mediation failed"
			};
		}
		try {
			if (decision.action === "fulfill") await route.fulfill(decision.fulfillment);
			else await route.abort("blockedbyclient");
		} catch {
			try {
				await route.abort("blockedbyclient");
			} catch {}
		}
		return decision;
	}
	return {
		exchange,
		mediate,
		handleRoute,
		remainingMs
	};
}
function safeHref(value) {
	try {
		return new URL(value).href;
	} catch {
		return String(value);
	}
}
function nextHop(location, base) {
	const absolute = /^[a-z][a-z0-9+.-]*:/i.test(location) ? location : location.startsWith("//") ? `${base.protocol}${location}` : null;
	if (absolute) {
		const reason = alternateIpReason(absolute);
		if (reason) return { error: reason };
	}
	try {
		return { href: new URL(location, base).href };
	} catch {
		return { error: "invalid url" };
	}
}
function describeRequest(request, mainPage) {
	const resourceType = typeof request.resourceType === "function" ? request.resourceType() : request.resourceType;
	const url = typeof request.url === "function" ? request.url() : request.url;
	let isPopup = Boolean(request.isPopup);
	try {
		const page = request.frame?.()?.page?.();
		if (page && mainPage && page !== mainPage) isPopup = true;
	} catch {}
	const disposition = typeof request.headers === "function" ? request.headers()["content-disposition"] : request.headers?.["content-disposition"];
	return {
		url,
		resourceType,
		isPopup,
		isWebSocket: request.isWebSocket || String(resourceType).toLowerCase() === "websocket",
		isDownload: Boolean(request.isDownload) || /attachment/i.test(String(disposition || "")),
		isServiceWorker: Boolean(request.isServiceWorker)
	};
}
function defaultLookup(hostname) {
	return lookup(hostname, { all: true });
}
async function defaultLaunchBrowser({ executablePath, timeoutMs }) {
	const { chromium } = await import("playwright");
	return chromium.launch({
		executablePath,
		headless: true,
		timeout: Math.min(timeoutMs || 2e4, 2e4),
		args: [
			"--disable-gpu",
			"--disable-extensions",
			"--no-first-run",
			"--no-default-browser-check",
			"--disable-background-networking",
			"--disable-sync",
			"--disable-component-update",
			"--mute-audio",
			"--disable-features=Translate,BackForwardCache,MediaRouter,DialMediaRouteProvider"
		]
	});
}
async function closeQuiet(resource) {
	try {
		await resource?.close?.();
	} catch {}
}
async function renderIsolated(browser, session, mediated, virtualTimeBudgetMs) {
	const context = await browser.newContext({
		...ISOLATED_CONTEXT_OPTIONS,
		proxy: { ...ISOLATED_CONTEXT_OPTIONS.proxy }
	});
	try {
		const page = await context.newPage();
		page.on?.("popup", (popup) => {
			popup?.close?.()?.catch?.(() => {});
		});
		page.on?.("download", (download) => {
			download?.cancel?.()?.catch?.(() => {});
		});
		context.on?.("page", (extra) => {
			if (extra !== page) extra?.close?.()?.catch?.(() => {});
		});
		await context.route?.("**/*", (route) => session.handleRoute(route, page));
		if (context.routeWebSocket) await context.routeWebSocket("**/*", (socket) => {
			socket.close();
		});
		const timeout = Math.max(1, session.remainingMs());
		await page.setContent(withBase(mediated.body.toString("utf8"), mediated.finalUrl), {
			waitUntil: "domcontentloaded",
			timeout
		});
		if (page.waitForLoadState) await page.waitForLoadState("networkidle", { timeout: virtualTimeBudgetMs }).catch(() => {});
		return page.content();
	} finally {
		await closeQuiet(context);
	}
}
function isRenderableDocument(mediated) {
	return mediated.ok && mediated.status === 200 && mediated.body?.length > 0 && htmlMime(mediated.headers) && !/attachment/i.test(String(headerValue(mediated.headers, "content-disposition") || ""));
}
/**
* Render `url` and return the serialized DOM plus the post-redirect URL.
* @returns {Promise<{html: string, finalUrl: string, origin: string} | null>}
*/
async function renderPage(url, { timeoutMs = 2e4, maxBytes = 5242880, virtualTimeBudgetMs = 8e3, maxRedirects = 5, maxRequests = 32, maxConcurrency = 4, edgePath, exists = existsSync, lookup = defaultLookup, get = pinnedGet, launchBrowser = defaultLaunchBrowser, now = Date.now } = {}) {
	let parsed;
	try {
		parsed = new URL(url);
	} catch {
		return null;
	}
	if (gateUrl(String(url)) || gateUrl(parsed.href)) return null;
	const binary = edgePath || findEdgeBinary(void 0, exists);
	if (!binary || !exists(binary)) return null;
	const session = createMediationSession({
		get,
		lookup,
		maxBytes,
		timeoutMs,
		maxRedirects,
		maxRequests,
		maxConcurrency,
		now
	});
	let mediated;
	try {
		mediated = await session.exchange(String(url));
	} catch {
		return null;
	}
	if (!isRenderableDocument(mediated)) return null;
	let browser;
	try {
		browser = await launchBrowser({
			executablePath: binary,
			timeoutMs
		});
	} catch (error) {
		error instanceof Error && error.message;
		return null;
	}
	try {
		const html = await renderIsolated(browser, session, mediated, virtualTimeBudgetMs);
		if (!html || html.length < 150) return null;
		return {
			html,
			finalUrl: mediated.finalUrl,
			origin: mediated.origin
		};
	} catch {
		return null;
	} finally {
		await closeQuiet(browser);
	}
}
//#endregion
//#region src/web/verification/claim_extractor.ts
/**
* Claim Extractor: Decomposes a synthesized answer into externally verifiable atomic claims.
*/
function extractAtomicClaims(answerText) {
	const rawSentences = answerText.replace(/^#+\s+.*$/gm, "").replace(/\[S\d+(?:\s*,\s*S\d+)*\]/gi, "").trim().split(/(?<=[.?!])\s+|\n+/).map((s) => s.replace(/^[•\-\*\d\.]+\s*/, "").trim()).filter((s) => s.length > 15);
	const claims = [];
	for (const sentence of rawSentences) {
		if (/^(based on current retrieved sources|I couldn't verify|Sources disagree:)/i.test(sentence) && !sentence.startsWith("Sources disagree:")) continue;
		if (/^(here is|according to|in summary|to summarize|overall|based on|as mentioned)/i.test(sentence) && sentence.length < 35) continue;
		claims.push(sentence);
	}
	return claims;
}
//#endregion
//#region src/web/verification/claim_verifier.ts
var ClaimVerifier = class {
	/**
	* Deterministic claim verification based on lexical & entity alignment with evidence.
	*/
	verifyClaimsDeterministic(claims, evidence) {
		const verifiedClaims = [];
		let supportedCount = 0;
		let unsupportedCount = 0;
		let conflictingCount = 0;
		for (const claim of claims) {
			let bestScore = 0;
			let matchingEvidence = null;
			let bestNumbersMatch = false;
			const tokenize = (s) => s.toLowerCase().replace(/[^\w\s]/g, " ").split(/\s+/).filter((t) => t.length > 1);
			const claimTokens = tokenize(claim);
			const claimNums = Array.from(claim.match(/\b\d+(?:\.\d+)?(?:k|m|b|t)?\b/gi) || []).map((n) => n.toLowerCase());
			for (const ev of evidence) {
				const evTokens = new Set(tokenize(ev.claim));
				const evNums = Array.from(ev.claim.match(/\b\d+(?:\.\d+)?(?:k|m|b|t)?\b/gi) || []).map((n) => n.toLowerCase());
				let matched = 0;
				for (const t of claimTokens) if (evTokens.has(t)) matched++;
				const coverage = claimTokens.length > 0 ? matched / claimTokens.length : 1;
				const jaccard = tokenJaccardSimilarity(claim, ev.claim);
				const combinedScore = Math.max(coverage, jaccard);
				const negative = (text) => /\b(not|never|no|cannot|doesn't|isn't|wasn't|unable)\b/i.test(text);
				const numsMatch = (claimNums.length === 0 || claimNums.every((n) => evNums.includes(n))) && negative(claim) === negative(ev.claim);
				if (combinedScore > bestScore) {
					bestScore = combinedScore;
					matchingEvidence = ev;
					bestNumbersMatch = numsMatch;
				}
			}
			let status = "UNSUPPORTED";
			let sources = [];
			const passage = matchingEvidence?.claim ?? "";
			const grounded = (matchingEvidence ? excerptContains(claim, passage) : false) && bestNumbersMatch && !quantitiesConflict(claim, passage) && !addsCurrentWording(claim, passage);
			const assessment = !matchingEvidence ? "unresolved" : matchingEvidence.status === "conflicting" ? "conflicting" : grounded ? "exact_excerpt" : bestScore >= .4 ? "lexical_match" : "unresolved";
			if (matchingEvidence) {
				if (matchingEvidence.status === "conflicting" || assessment === "conflicting") {
					status = "CONFLICTING";
					sources = matchingEvidence.supportingSources;
					conflictingCount++;
				} else if (grounded) {
					status = "SUPPORTED";
					sources = matchingEvidence.supportingSources;
					supportedCount++;
				} else {
					status = "UNSUPPORTED";
					unsupportedCount++;
				}
			} else {
				status = "UNSUPPORTED";
				unsupportedCount++;
			}
			verifiedClaims.push({
				claim,
				status,
				sources,
				assessment
			});
		}
		return {
			claims: verifiedClaims,
			allSupported: unsupportedCount === 0 && conflictingCount === 0,
			supportedCount,
			unsupportedCount,
			conflictingCount
		};
	}
};
function normalizeText(value) {
	return value.toLowerCase().replace(/[^\w\s.%$€]/g, " ").replace(/(?<!\d)\.(?!\d)/g, " ").replace(/\s+/g, " ").trim();
}
var TRAILING_UNIT = /^(?:[kmbt]|kilometers?|km|miles?|mi|dollars?|usd|cents?|euros?|gb|mb|kb|tb|percent|watts?)(?!\w)/;
var CURRENT_MARKERS = [
	"as of now",
	"right now",
	"currently",
	"nowadays",
	"presently",
	"current",
	"today",
	"latest"
];
/** The quoted text has to occur in the passage. Overlap is not enough. */
function excerptContains(quote, passage) {
	const quoted = normalizeText(quote);
	const source = normalizeText(passage);
	if (!quoted) return false;
	let from = 0;
	while (from < source.length) {
		const at = source.indexOf(quoted, from);
		if (at < 0) return false;
		const end = at + quoted.length;
		const startsToken = at === 0 || source[at - 1] === " ";
		const endsToken = end === source.length || source[end] === " ";
		if (startsToken && endsToken && !TRAILING_UNIT.test(source.slice(end).trimStart())) return true;
		from = at + 1;
	}
	return false;
}
function canonicalUnit(symbol, raw) {
	if (symbol === "$" || /^(?:dollars?|usd)$/i.test(raw)) return "dollar";
	if (symbol === "€" || /^euros?$/i.test(raw)) return "euro";
	if (raw === "%" || /^percent$/i.test(raw)) return "percent";
	if (/^(?:kilometers?|km)$/i.test(raw)) return "km";
	if (/^(?:miles?|mi)$/i.test(raw)) return "mile";
	if (/^cents?$/i.test(raw)) return "cent";
	if (/^watts?$/i.test(raw)) return "watt";
	if (/^(?:gb|mb|kb|tb)$/i.test(raw)) return raw.toLowerCase();
	return null;
}
function extractQuantities(text) {
	const found = [];
	for (const match of text.matchAll(/(?<sym>[$€])?\s*\b(?<num>\d+(?:\.\d+)?)(?<suf>[kmbt])?\b(?:\s*(?<unit>kilometers?|km|miles?|mi|dollars?|usd|cents?|euros?|gb|mb|kb|tb|percent|%|watts?)(?!\w))?/gi)) {
		const num = match.groups?.num;
		if (!num) continue;
		const suffix = (match.groups?.suf ?? "").toLowerCase();
		found.push({
			value: `${num}${suffix}`.toLowerCase(),
			unit: canonicalUnit(match.groups?.sym ?? "", match.groups?.unit ?? "")
		});
	}
	return found;
}
/** Same magnitude with a different or dropped unit is not the same quantity. */
function quantitiesConflict(claim, passage) {
	const passageQuantities = extractQuantities(passage);
	for (const quantity of extractQuantities(claim)) {
		const sameValue = passageQuantities.filter((item) => item.value === quantity.value);
		if (quantity.unit) {
			if (!sameValue.some((item) => item.unit === quantity.unit)) return true;
		} else if (sameValue.length > 0 && sameValue.every((item) => item.unit)) return true;
	}
	return false;
}
function hasMarker(text, marker) {
	const escaped = marker.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
	return new RegExp(`(?:^|\\s)${escaped}(?=\\s|$)`).test(text);
}
/** "Currently" / "latest" in the claim is not supported by an older passage that lacks it. */
function addsCurrentWording(claim, passage) {
	const claimed = normalizeText(claim);
	const source = normalizeText(passage);
	return CURRENT_MARKERS.some((marker) => hasMarker(claimed, marker) && !hasMarker(source, marker));
}
//#endregion
//#region src/web/verticals/weather/weather_provider.ts
var weatherTemperature = (celsius, unit) => unit === "fahrenheit" ? celsius * 9 / 5 + 32 : celsius;
function validateWeatherRequest(location, forecastDays = 3) {
	if (typeof location !== "string" || !location.trim() || location.length > 200) throw new Error("Weather requires an explicit location of 1–200 characters.");
	if (typeof forecastDays !== "number" || !Number.isInteger(forecastDays) || forecastDays < 1 || forecastDays > 7) throw new Error("forecastDays must be an integer from 1 to 7.");
	return {
		location: location.trim(),
		forecastDays
	};
}
function record(value, label) {
	if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`Weather response has invalid ${label}.`);
	return value;
}
function text(value, label) {
	if (typeof value !== "string" || !value.trim() || value.length > 500) throw new Error(`Weather response has missing ${label}.`);
	return value.trim();
}
function measurement(value, label, min = -Infinity, max = Infinity) {
	if (typeof value !== "number" || !Number.isFinite(value) || value < min || value > max) throw new Error(`Weather response has missing or invalid ${label}.`);
	return value;
}
async function requestJson(url, label) {
	const res = await fetch(url, {
		signal: AbortSignal.timeout(8e3),
		redirect: "error"
	});
	if (!res.ok) throw new Error(`${label} unavailable (HTTP ${res.status}).`);
	const reader = res.body?.getReader();
	if (!reader) throw new Error(`${label} returned an empty response.`);
	const chunks = [];
	let size = 0;
	try {
		while (true) {
			const { done, value } = await reader.read();
			if (done) break;
			size += value.byteLength;
			if (size > 1048576) throw new Error(`${label} response exceeds 1 MiB.`);
			chunks.push(value);
		}
	} finally {
		await reader.cancel();
	}
	const bytes = new Uint8Array(size);
	let offset = 0;
	for (const chunk of chunks) {
		bytes.set(chunk, offset);
		offset += chunk.byteLength;
	}
	const data = record(JSON.parse(new TextDecoder().decode(bytes)), label);
	if (data.error === true) throw new Error(`${label} unavailable: ${typeof data.reason === "string" ? data.reason.slice(0, 300) : "provider error"}.`);
	return data;
}
var WeatherProvider = class {
	extractLocation(query) {
		const location = (query.match(/(?:weather(?:\s+report)?|temperature|forecast)(?:\s+(?:in|for|at))\s+([^?!]+)/i) || query.match(/(?:in|for|at)\s+([^?!]+)/i) || query.match(/^([^?!]+)\s+weather/i))?.[1].replace(/\b(today|tomorrow|now|tonight|this week|current)\b/gi, "").replace(/[.]+$/, "").trim();
		if (!location) throw new Error("Please specify a location for weather.");
		return location;
	}
	async geocode(name) {
		const parts = name.split(",").map((part) => part.trim());
		if (parts.some((part) => !part) || parts.length > 3) throw new Error(`Could not resolve all location qualifiers: ${name}. Try City, country or City, state.`);
		const data = await requestJson(`https://geocoding-api.open-meteo.com/v1/search?name=${encodeURIComponent(parts[0])}&count=10&language=en&format=json`, "Geocoding");
		if (!Array.isArray(data.results) || !data.results.length) throw new Error(`Could not resolve location: ${name}. Try City, country or City, state.`);
		const resolved = data.results.map((value) => record(value, "location"));
		const normalizeName = (value) => value.normalize("NFKD").replace(/\p{M}/gu, "").toLocaleLowerCase("en");
		const exact = resolved.filter((candidate) => typeof candidate.name === "string" && normalizeName(candidate.name) === normalizeName(parts[0]));
		const candidates = (exact.length ? exact : resolved).filter((candidate) => parts.slice(1).every((part) => [
			candidate.country,
			candidate.country_code,
			candidate.admin1,
			candidate.admin2
		].some((value) => typeof value === "string" && value.toLocaleLowerCase("en") === part.toLocaleLowerCase("en"))));
		if (!candidates.length) throw new Error(`Could not resolve all location qualifiers: ${name}. Try City, country or City, state.`);
		const distinct = new Map(candidates.map((candidate) => [`${candidate.name}|${candidate.admin1}|${candidate.country}`, candidate]));
		if (distinct.size > 1) throw new Error(`Ambiguous location. Specify a city and region or country: ${[...distinct.values()].slice(0, 5).map((candidate) => [
			candidate.name,
			candidate.admin1,
			candidate.country
		].filter(Boolean).join(", ")).join("; ")}.`);
		const first = candidates[0];
		return {
			name: text(first.name, "location name"),
			region: typeof first.admin1 === "string" ? first.admin1 : void 0,
			country: typeof first.country === "string" ? first.country : void 0,
			lat: measurement(first.latitude, "latitude", -90, 90),
			lon: measurement(first.longitude, "longitude", -180, 180)
		};
	}
	async execute(query) {
		return this.executeLocation(this.extractLocation(query));
	}
	async executeLocation(location, forecastDays = 3, temperatureUnit = "celsius") {
		const request = validateWeatherRequest(location, forecastDays);
		if (temperatureUnit !== "celsius" && temperatureUnit !== "fahrenheit") throw new Error("Temperature unit must be celsius or fahrenheit.");
		const coords = await this.geocode(request.location);
		const sourceUrl = `https://api.open-meteo.com/v1/forecast?latitude=${coords.lat}&longitude=${coords.lon}&current=temperature_2m,relative_humidity_2m,apparent_temperature,wind_speed_10m,weather_code,precipitation&hourly=temperature_2m,apparent_temperature,relative_humidity_2m,precipitation_probability,wind_speed_10m,weather_code&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max&temperature_unit=celsius&wind_speed_unit=kmh&timezone=auto&forecast_days=${request.forecastDays}`;
		const data = await requestJson(sourceUrl, "Weather");
		const cur = record(data.current, "current conditions");
		const observedAt = text(cur.time, "current timestamp");
		if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(observedAt) || !Number.isFinite(Date.parse(observedAt))) throw new Error("Weather response has invalid current timestamp.");
		const timezone = text(data.timezone, "timezone");
		try {
			new Intl.DateTimeFormat("en", { timeZone: timezone });
		} catch {
			throw new Error("Weather response has invalid timezone.");
		}
		const daily = record(data.daily, "daily forecast");
		if (!Array.isArray(daily.time) || !daily.time.length) throw new Error("Weather response has missing forecast dates.");
		const min = daily.temperature_2m_min, max = daily.temperature_2m_max, rain = daily.precipitation_probability_max;
		if (![
			min,
			max,
			rain
		].every(Array.isArray)) throw new Error("Weather response has missing forecast measurements.");
		const forecast = [];
		for (let i = 0; i < Math.min(daily.time.length, request.forecastDays); i++) {
			const date = text(daily.time[i], "forecast date");
			if (!/^\d{4}-\d{2}-\d{2}$/.test(date) || !Number.isFinite(Date.parse(date))) throw new Error("Weather response has invalid forecast date.");
			const minC = measurement(min[i], "minimum temperature");
			const maxC = measurement(max[i], "maximum temperature", minC);
			forecast.push({
				date,
				minC,
				maxC,
				precipitationProbabilityPercent: measurement(rain[i], "rain probability", 0, 100)
			});
		}
		const hourly = [];
		if (data.hourly) {
			const hours = record(data.hourly, "hourly forecast");
			if (!Array.isArray(hours.time)) throw new Error("Weather response has missing hourly times.");
			for (let i = 0; i < hours.time.length && hourly.length < 48; i++) {
				const time = text(hours.time[i], "hourly timestamp");
				if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(time)) throw new Error("Weather response has invalid hourly time.");
				if (time < observedAt.slice(0, 13) + ":00") continue;
				const at = (key, min = -Infinity, max = Infinity) => measurement(hours[key]?.[i], key, min, max);
				hourly.push({
					time,
					temperatureC: at("temperature_2m"),
					apparentTemperatureC: at("apparent_temperature"),
					humidityPercent: at("relative_humidity_2m", 0, 100),
					precipitationProbabilityPercent: at("precipitation_probability", 0, 100),
					windKph: at("wind_speed_10m", 0),
					weatherCode: at("weather_code", 0, 99)
				});
			}
		}
		return {
			sourceUrl,
			observedAt,
			timezone,
			fetchedAt: (/* @__PURE__ */ new Date()).toISOString(),
			temperatureUnit,
			hourly,
			units: {
				temperature: "°C",
				windSpeed: "km/h",
				humidity: "%",
				precipitationProbability: "%"
			},
			location: {
				name: coords.name,
				region: coords.region,
				country: coords.country,
				latitude: coords.lat,
				longitude: coords.lon
			},
			current: {
				temperatureC: measurement(cur.temperature_2m, "temperature"),
				apparentTemperatureC: measurement(cur.apparent_temperature, "apparent temperature"),
				humidityPercent: measurement(cur.relative_humidity_2m, "humidity", 0, 100),
				windKph: measurement(cur.wind_speed_10m, "wind speed", 0),
				weatherCode: cur.weather_code === void 0 ? void 0 : measurement(cur.weather_code, "conditions", 0, 99),
				precipitationMm: cur.precipitation === void 0 ? void 0 : measurement(cur.precipitation, "precipitation", 0)
			},
			forecast
		};
	}
	formatReportAsEvidence(report) {
		const loc = report.location, cur = report.current;
		const unit = report.temperatureUnit === "fahrenheit" ? "°F" : "°C";
		const temp = (value) => `${weatherTemperature(value, report.temperatureUnit).toFixed(1)}${unit}`;
		return `Location: ${[
			loc.name,
			loc.region,
			loc.country
		].filter(Boolean).join(", ")}\nAs of ${report.observedAt} (${report.timezone}), Open-Meteo model data.\nCurrent Temperature: ${temp(cur.temperatureC)} (Feels like ${temp(cur.apparentTemperatureC)})\nHumidity: ${cur.humidityPercent}%\nWind Speed: ${cur.windKph} km/h\n` + report.forecast.map((f) => `${f.date}: High ${temp(f.maxC)}, Low ${temp(f.minC)}, Rain probability ${f.precipitationProbabilityPercent}%`).join("\n");
	}
};
//#endregion
//#region scripts/web-worker.mjs
/** Bounded page shape returned to the model for visit/fetch-url calls. */
function shapeDocument(doc, selection = {}) {
	const text = doc.text || "";
	let end = Math.min(8e3, text.length);
	if (end < text.length && /[\uDC00-\uDFFF]/.test(text[end])) end--;
	const result = {
		id: doc.id,
		url: doc.url,
		title: doc.title,
		extractionMethod: doc.metadata?.extractionMethod || "main_content",
		snippetOnly: (doc.metadata?.extractionMethod || "").includes("snippet"),
		ocrRequired: Boolean(doc.metadata?.ocrRequired),
		text: text.slice(0, end),
		textOffset: 0,
		nextTextOffset: end < text.length ? end : null,
		textTotalChars: text.length,
		textTruncated: end < text.length,
		headings: (doc.headings || []).slice(0, 30),
		links: (doc.links || []).slice(0, 20)
	};
	if ([
		"page",
		"section",
		"passage",
		"offset",
		"textOffset"
	].some((key) => selection[key] != null)) {
		const store = new DocumentStore();
		store.save({
			...doc,
			fullText: doc.text || "",
			extractionMethod: result.extractionMethod
		});
		delete result.text;
		delete result.textTruncated;
		delete result.textOffset;
		delete result.nextTextOffset;
		result.passages = store.open(doc.id, selection);
	}
	return result;
}
/** Persist a fetched document into a research session so open/find can
* revisit it later. Sessions are sqlite rows keyed by id; unknown ids start a
* new minimal session whose only content is this document. */
function stashDocument(storage, sessionId, doc) {
	if (typeof sessionId !== "string" || !sessionId) throw new Error("store needs a sessionId");
	if (!doc || typeof doc.id !== "string" || typeof doc.url !== "string") throw new Error("store needs a fetched document");
	let session;
	try {
		session = storage.load(sessionId);
	} catch {
		session = {
			id: sessionId,
			documents: []
		};
	}
	const documents = Array.isArray(session.documents) ? session.documents : [];
	if (!documents.some((d) => d.id === doc.id)) documents.push(doc);
	session.documents = documents;
	storage.save(session);
	return {
		sessionId,
		documentId: doc.id,
		url: doc.url,
		title: doc.title
	};
}
function loadDocumentStore(storage, sessionId) {
	const store = new DocumentStore();
	try {
		const session = storage.load(sessionId);
		for (const doc of session.documents || []) store.save({
			...doc,
			fullText: doc.text || "",
			pages: doc.pages,
			extractionMethod: doc.metadata?.extractionMethod || "main_content"
		});
	} catch {}
	return store;
}
var input = "";
for await (const chunk of process.stdin) {
	input += chunk;
	if (input.length > 64e3) throw new Error("Research input too large");
}
var storage;
async function runRequest(request) {
	if (request.action === "weather") {
		validateWeatherRequest(request.location, request.forecastDays);
		if (request.temperatureUnit !== void 0 && !["celsius", "fahrenheit"].includes(request.temperatureUnit)) throw new Error("Temperature unit must be celsius or fahrenheit.");
		const provider = new WeatherProvider();
		const cache = request.weatherCachePath ? new SqliteStorage(request.weatherCachePath) : null;
		try {
			const key = JSON.stringify([
				String(request.location || "").trim().toLocaleLowerCase("en"),
				request.forecastDays ?? 3,
				request.temperatureUnit ?? "celsius"
			]);
			const cached = await cache?.get("weather-v2", key);
			if (cached) return {
				...cached,
				cached: true
			};
			const report = await provider.executeLocation(request.location, request.forecastDays, request.temperatureUnit);
			const result = {
				...report,
				cached: false,
				evidence: provider.formatReportAsEvidence(report)
			};
			await cache?.set("weather-v2", key, result, 600);
			return result;
		} finally {
			cache?.close();
		}
	}
	storage = new SqliteStorage(request.databasePath || ":memory:");
	if (request.action === "clear-cache") {
		await storage.clear();
		if (request.weatherCachePath) {
			const weatherCache = new SqliteStorage(request.weatherCachePath);
			try {
				await weatherCache.clear();
			} finally {
				weatherCache.close();
			}
		}
		return { cleared: true };
	}
	const emitProgress = (progress) => {
		try {
			process.stderr.write(`\x1e${JSON.stringify(progress)}\n`);
		} catch {}
	};
	const engine = new WebSearchEngine({
		config: request.config,
		storage,
		onProgress: emitProgress,
		fetcher: new PinnedPageFetcher(request.config?.fetch?.globalConcurrency, request.config?.fetch?.perDomainConcurrency),
		jsRender: request.config?.fetch?.jsRenderFallback ? (url) => renderPage(url, {
			timeoutMs: request.config.fetch.jsRenderTimeoutMs || 2e4,
			maxBytes: request.config.fetch.maxBytes
		}) : void 0
	});
	let result;
	if (request.action === "health") {
		result = await engine.getHealthStatus();
		result.database = "sqlite_ready";
	} else if (request.action === "open" || request.action === "find") {
		const doc = storage.load(request.sessionId).documents.find((d) => d.id === request.documentId);
		if (!doc) throw new Error("Unknown document in this session");
		const store = loadDocumentStore(storage, request.sessionId);
		if (request.action === "open") {
			const passages = store.open(request.documentId, {
				page: request.page,
				section: request.section,
				passage: request.passage,
				offset: request.offset,
				textOffset: request.textOffset
			});
			result = {
				id: doc.id,
				sessionId: request.sessionId,
				documentId: doc.id,
				url: doc.url,
				title: doc.title,
				extractionMethod: doc.metadata?.extractionMethod || "main_content",
				snippetOnly: (doc.metadata?.extractionMethod || "").includes("snippet"),
				passages,
				links: (doc.links || []).slice(0, 20),
				headings: (doc.headings || []).slice(0, 30)
			};
		} else {
			const passages = store.find(request.documentId, String(request.term || ""), 400);
			result = {
				id: doc.id,
				sessionId: request.sessionId,
				documentId: doc.id,
				url: doc.url,
				title: doc.title,
				extractionMethod: doc.metadata?.extractionMethod || "main_content",
				snippetOnly: (doc.metadata?.extractionMethod || "").includes("snippet"),
				passages
			};
		}
	} else if (request.action === "search") result = await engine.searchQueries(request.query);
	else if (request.action === "store" || request.action === "fetch-url") {
		if (request.action === "store") result = stashDocument(storage, request.sessionId, request.document);
		else {
			const doc = await engine.fetchUrl(request.url);
			if (typeof request.sessionId === "string" && request.sessionId) {
				stashDocument(storage, request.sessionId, doc);
				result = {
					...shapeDocument(doc, request),
					sessionId: request.sessionId,
					documentId: doc.id
				};
			} else result = shapeDocument(doc, request);
		}
	} else if (request.action === "fetch-urls") {
		const urls = (Array.isArray(request.urls) ? request.urls : []).slice(0, 4);
		result = { pages: await Promise.all(urls.map(async (url) => {
			try {
				return shapeDocument(await engine.fetchUrl(String(url)));
			} catch (error) {
				return {
					url: String(url),
					isError: true,
					message: String(error?.message || error)
				};
			}
		})) };
	} else if (request.action === "verify") {
		const claims = extractAtomicClaims(String(request.answer || "").slice(0, 16e3));
		const evidence = (Array.isArray(request.evidence) ? request.evidence : []).slice(0, 30).map((item, index) => ({
			id: `E${index + 1}`,
			claim: String(item?.claim ?? item?.text ?? "").slice(0, 8e3),
			supportingSources: [String(item?.url || `E${index + 1}`)],
			status: "supported",
			confidence: 1
		})).filter((item) => item.claim.trim().length > 0);
		result = new ClaimVerifier().verifyClaimsDeterministic(claims, evidence);
	} else {
		const session = await engine.research(request.question, {
			mode: request.mode,
			freshness: request.freshness,
			language: request.language,
			category: request.category
		});
		storage.save(session);
		result = request.full ? session : {
			id: session.id,
			question: session.question,
			answer: session.answer,
			route: session.route,
			sources: session.sources,
			evidence: session.evidence,
			verification: session.verification,
			tokenUsage: session.tokenUsage,
			trace: session.trace,
			startedAt: session.startedAt,
			completedAt: session.completedAt,
			documents: session.documents.map((d) => ({
				id: d.id,
				title: d.title,
				url: d.url
			}))
		};
		if (request.trace) process.stderr.write(formatDiagnosticReport(session) + "\n");
	}
	return result;
}
try {
	const result = await runRequest(JSON.parse(input));
	process.stdout.write(JSON.stringify(result));
} catch (error) {
	process.stderr.write(String(error.message || error) + "\n");
	process.exitCode = 1;
} finally {
	storage?.close();
}
//#endregion
export {};
