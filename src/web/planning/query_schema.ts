/**
 * Query Planner Schema: Structured schema definition for query planning.
 */

export const QUERY_PLAN_SCHEMA = {
  type: 'object',
  properties: {
    queries: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          query: { type: 'string', description: 'Search terms to query' },
          purpose: { type: 'string', description: 'Information sought by this search' },
          freshness: {
            type: 'string',
            enum: ['day', 'week', 'month', 'year', 'any'],
            description: 'Temporal freshness constraint',
          },
        },
        required: ['query', 'purpose'],
      },
      minItems: 1,
      maxItems: 4,
    },
  },
  required: ['queries'],
};
