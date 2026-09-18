/**
 * Source Quality Model: Categorizes sources by domain and pattern to compute
 * authority score and prioritize official/primary documentation over unverified material.
 */

import type { SourceType } from '../types';

interface DomainRule {
  type: SourceType;
  baseScore: number;
}

const DOMAIN_RULES: Record<string, DomainRule> = {
  // Official documentation & code registries
  'docs.': { type: 'official_documentation', baseScore: 0.95 },
  'developer.': { type: 'official_documentation', baseScore: 0.95 },
  'github.com': { type: 'official_documentation', baseScore: 0.90 },
  'gitlab.com': { type: 'official_documentation', baseScore: 0.88 },
  'huggingface.co': { type: 'official_documentation', baseScore: 0.92 },
  'npmjs.com': { type: 'official_documentation', baseScore: 0.90 },
  'pypi.org': { type: 'official_documentation', baseScore: 0.90 },
  'crates.io': { type: 'official_documentation', baseScore: 0.90 },
  'react.dev': { type: 'official_documentation', baseScore: 0.95 },
  'nextjs.org': { type: 'official_documentation', baseScore: 0.95 },
  'python.org': { type: 'official_documentation', baseScore: 0.95 },
  'rust-lang.org': { type: 'official_documentation', baseScore: 0.95 },
  'go.dev': { type: 'official_documentation', baseScore: 0.95 },
  'w3.org': { type: 'official_documentation', baseScore: 0.95 },
  'mozilla.org': { type: 'official_documentation', baseScore: 0.95 },

  // Academic sources
  'arxiv.org': { type: 'academic_paper', baseScore: 0.95 },
  'biorxiv.org': { type: 'academic_paper', baseScore: 0.93 },
  'nature.com': { type: 'academic_paper', baseScore: 0.95 },
  'science.org': { type: 'academic_paper', baseScore: 0.95 },
  'acm.org': { type: 'academic_paper', baseScore: 0.92 },
  'ieee.org': { type: 'academic_paper', baseScore: 0.92 },
  'openreview.net': { type: 'academic_paper', baseScore: 0.92 },
  'nih.gov': { type: 'academic_paper', baseScore: 0.95 },

  // Primary news & tech journalism
  'reuters.com': { type: 'news_organization', baseScore: 0.85 },
  'apnews.com': { type: 'news_organization', baseScore: 0.85 },
  'bbc.com': { type: 'news_organization', baseScore: 0.85 },
  'theverge.com': { type: 'specialist_publication', baseScore: 0.78 },
  'techcrunch.com': { type: 'specialist_publication', baseScore: 0.78 },
  'arstechnica.com': { type: 'specialist_publication', baseScore: 0.82 },
  'wired.com': { type: 'specialist_publication', baseScore: 0.78 },

  // Community discussion & forums
  'stackoverflow.com': { type: 'community_forum', baseScore: 0.70 },
  'stackexchange.com': { type: 'community_forum', baseScore: 0.68 },
  'news.ycombinator.com': { type: 'community_forum', baseScore: 0.65 },
  'reddit.com': { type: 'community_forum', baseScore: 0.55 },
  'medium.com': { type: 'specialist_publication', baseScore: 0.50 },
  'dev.to': { type: 'specialist_publication', baseScore: 0.55 },

  // Social
  'twitter.com': { type: 'social_media', baseScore: 0.40 },
  'x.com': { type: 'social_media', baseScore: 0.40 },
  'linkedin.com': { type: 'social_media', baseScore: 0.45 },
};

export interface SourceQualityAssessment {
  sourceType: SourceType;
  authorityScore: number;
  isPrimary: boolean;
}

export function assessSourceQuality(urlStr: string, title?: string): SourceQualityAssessment {
  let domain = 'unknown';
  let path = '';
  try {
    const url = new URL(urlStr);
    domain = url.hostname.toLowerCase();
    if (domain.startsWith('www.')) domain = domain.substring(4);
    path = url.pathname.toLowerCase();
  } catch {
    // fallback
  }

  // Check prefix rules (e.g. docs.*, developer.*)
  for (const [pattern, rule] of Object.entries(DOMAIN_RULES)) {
    if (pattern.endsWith('.') && domain.startsWith(pattern)) {
      return {
        sourceType: rule.type,
        authorityScore: rule.baseScore,
        isPrimary: rule.type === 'official_documentation' || rule.type === 'primary_source',
      };
    }
  }

  // Check exact domain or subdomain match
  for (const [targetDomain, rule] of Object.entries(DOMAIN_RULES)) {
    if (!targetDomain.endsWith('.') && (domain === targetDomain || domain.endsWith(`.${targetDomain}`))) {
      return {
        sourceType: rule.type,
        authorityScore: rule.baseScore,
        isPrimary: rule.type === 'official_documentation' || rule.type === 'academic_paper',
      };
    }
  }

  // Heuristics based on path or title
  const lowerTitle = (title || '').toLowerCase();
  if (
    path.includes('/docs/') ||
    path.includes('/documentation/') ||
    path.includes('/api/') ||
    path.includes('/releases/') ||
    lowerTitle.includes('official documentation') ||
    lowerTitle.includes('release notes')
  ) {
    return {
      sourceType: 'official_documentation',
      authorityScore: 0.85,
      isPrimary: true,
    };
  }

  // TLD heuristics (.edu, .gov)
  if (domain.endsWith('.edu') || domain.endsWith('.gov') || domain.endsWith('.ac.uk')) {
    return {
      sourceType: 'academic_paper',
      authorityScore: 0.90,
      isPrimary: true,
    };
  }

  // Default unknown source
  return {
    sourceType: 'unknown',
    authorityScore: 0.50,
    isPrimary: false,
  };
}
