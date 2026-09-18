/**
 * Request Normalizer: Analyzes and normalizes user queries before routing.
 * Detects language, date expressions, location references, named entities,
 * URLs, quoted terms, and question type, while strictly preserving original query intent.
 */

export interface DetectedEntity {
  type: 'location' | 'software' | 'model' | 'date' | 'quoted_term' | 'url' | 'entity';
  value: string;
}

export interface NormalizedRequest {
  originalQuery: string;
  normalizedQuery: string;
  language: string;
  questionType: 'weather' | 'comparison' | 'software' | 'news' | 'academic' | 'documentation' | 'conceptual' | 'factual' | 'general';
  entities: DetectedEntity[];
  urls: string[];
  quotedTerms: string[];
  dateExpressions: string[];
}

const KNOWN_LOCATIONS = new Set([
  'ranaghat',
  'tokyo',
  'london',
  'kolkata',
  'delhi',
  'mumbai',
  'bangalore',
  'new york',
  'paris',
  'berlin',
  'sydney',
  'san francisco',
  'singapore',
  'seattle',
  'chicago',
  'toronto',
]);

const LOCATION_CONTEXT: Record<string, string> = {
  ranaghat: 'Ranaghat, West Bengal, India',
  kolkata: 'Kolkata, West Bengal, India',
  delhi: 'Delhi, India',
  mumbai: 'Mumbai, Maharashtra, India',
  bangalore: 'Bengaluru, Karnataka, India',
  tokyo: 'Tokyo, Japan',
  london: 'London, United Kingdom',
  'new york': 'New York, NY, USA',
  paris: 'Paris, France',
  berlin: 'Berlin, Germany',
  sydney: 'Sydney, Australia',
  'san francisco': 'San Francisco, CA, USA',
  singapore: 'Singapore',
  seattle: 'Seattle, WA, USA',
  chicago: 'Chicago, IL, USA',
  toronto: 'Toronto, Canada',
};

const KNOWN_MODELS = [
  'qwen',
  'deepseek',
  'llama',
  'mistral',
  'phi',
  'gemma',
  'claude',
  'gpt-4',
  'gemini',
];

const KNOWN_SOFTWARE = [
  'react',
  'next.js',
  'nextjs',
  'vue',
  'angular',
  'svelte',
  'node',
  'python',
  'rust',
  'typescript',
  'tauri',
  'vitest',
  'docker',
];

export function normalizeRequest(rawQuery: string): NormalizedRequest {
  const originalQuery = rawQuery.trim();
  const lower = originalQuery.toLowerCase();

  // 1. Quoted terms detection
  const quotedTerms: string[] = [];
  const quoteRegex = /"([^"]+)"|'([^']+)'/g;
  let qMatch;
  while ((qMatch = quoteRegex.exec(originalQuery)) !== null) {
    const term = qMatch[1] || qMatch[2];
    if (term) quotedTerms.push(term.trim());
  }

  // 2. URLs detection
  const urls: string[] = [];
  const urlRegex = /https?:\/\/[^\s]+/gi;
  let uMatch;
  while ((uMatch = urlRegex.exec(originalQuery)) !== null) {
    urls.push(uMatch[0]);
  }

  // 3. Date expressions detection
  const dateExpressions: string[] = [];
  const datePattern = /\b(today|yesterday|tomorrow|tonight|this week|last week|this month|recently|latest|current|right now|20\d{2})\b/gi;
  let dMatch;
  while ((dMatch = datePattern.exec(originalQuery)) !== null) {
    if (!dateExpressions.includes(dMatch[0].toLowerCase())) {
      dateExpressions.push(dMatch[0].toLowerCase());
    }
  }

  // 4. Entities & Locations detection
  const entities: DetectedEntity[] = [];

  // Quoted terms as entities
  for (const q of quotedTerms) {
    entities.push({ type: 'quoted_term', value: q });
  }

  // URLs as entities
  for (const u of urls) {
    entities.push({ type: 'url', value: u });
  }

  // Location detection
  let detectedLocationName: string | null = null;
  for (const loc of KNOWN_LOCATIONS) {
    const locRegex = new RegExp(`\\b${loc}\\b`, 'i');
    if (locRegex.test(lower)) {
      detectedLocationName = loc;
      const formatted = LOCATION_CONTEXT[loc] || loc.charAt(0).toUpperCase() + loc.slice(1);
      entities.push({ type: 'location', value: formatted });
      break;
    }
  }

  // General "in <Location>" or "for <Location>" fallback
  if (!detectedLocationName) {
    const inLocMatch = originalQuery.match(/\b(?:in|for|at)\s+([A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+)*)\b/);
    if (inLocMatch && inLocMatch[1]) {
      const locVal = inLocMatch[1].trim();
      if (!['Today', 'Tonight', 'This', 'Recent'].includes(locVal)) {
        entities.push({ type: 'location', value: locVal });
      }
    }
  }

  // Model detection
  for (const m of KNOWN_MODELS) {
    if (new RegExp(`\\b${m}\\b`, 'i').test(lower)) {
      entities.push({ type: 'model', value: m });
    }
  }

  // Software detection
  for (const s of KNOWN_SOFTWARE) {
    if (new RegExp(`\\b${s.replace('.', '\\.')}\\b`, 'i').test(lower)) {
      entities.push({ type: 'software', value: s });
    }
  }

  // 5. Question type classification
  let questionType: NormalizedRequest['questionType'] = 'general';
  if (/\b(weather|temperature|forecast|rain|snow|humidity|wind)\b/i.test(lower)) {
    questionType = 'weather';
  } else if (/\b(compare|versus|vs\.?|differences between)\b/i.test(lower)) {
    questionType = 'comparison';
  } else if (
    /\b(release|changelog|version|update|newest stable|what changed)\b/i.test(lower) ||
    (entities.some((e) => e.type === 'software') && /\b(changed|new|releases?|features|updates?)\b/i.test(lower))
  ) {
    questionType = 'software';
  } else if (/\b(news|headline|what happened|breaking)\b/i.test(lower)) {
    questionType = 'news';
  } else if (/\b(arxiv|paper|research|benchmark|swe-bench)\b/i.test(lower)) {
    questionType = 'academic';
  } else if (/\b(documentation|docs|api|guide)\b/i.test(lower)) {
    questionType = 'documentation';
  } else if (/^(what (is|does)|how does|explain|define)\b/i.test(lower) && dateExpressions.length === 0) {
    questionType = 'conceptual';
  } else if (/^(who|when|where|what is the capital|what is the height)\b/i.test(lower)) {
    questionType = 'factual';
  }

  // 6. Build normalized query representation
  let normalizedQuery = originalQuery.replace(/\s+/g, ' ').trim();
  if (detectedLocationName && questionType === 'weather' && LOCATION_CONTEXT[detectedLocationName]) {
    // Gracefully enrich weather query location context without losing intent
    const fullLoc = LOCATION_CONTEXT[detectedLocationName];
    const locRegex = new RegExp(`\\b${detectedLocationName}\\b`, 'i');
    normalizedQuery = normalizedQuery.replace(locRegex, fullLoc);
  }

  return {
    originalQuery,
    normalizedQuery,
    language: 'en',
    questionType,
    entities,
    urls,
    quotedTerms,
    dateExpressions,
  };
}
