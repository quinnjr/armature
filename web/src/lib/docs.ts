export interface DocMetadata {
  /** URL slug under /docs/ */
  id: string;
  title: string;
  category: string;
  /**
   * Markdown source in the repository's docs/ directory. Defaults to `<id>.md`.
   * `null` means the page has no markdown source (rendered as a custom page or placeholder).
   */
  file?: string | null;
  /** Rendered from a hand-built Astro component instead of markdown. */
  hasComponent?: boolean;
  /**
   * Meta description override. Omit to derive one from the markdown's first
   * prose paragraph (see `docmeta.ts`) — only set this when the derived text
   * reads badly, e.g. for component-rendered pages with no markdown source.
   */
  description?: string;
}

// Documentation metadata with user-friendly titles.
// Category order here controls sidebar order.
export const docs: DocMetadata[] = [
  // 1. Getting Started - First things first
  {
    id: 'readme',
    title: 'Overview',
    category: 'Getting Started',
    file: 'README.md',
    hasComponent: true,
    description:
      'Armature is a batteries-included Rust web framework with NestJS-style dependency injection, decorator macros, modules, guards and interceptors. Start here.',
  },
  { id: 'micro-framework-guide', title: 'Micro-Framework Mode', category: 'Getting Started' },
  { id: 'project-templates', title: 'Project Templates', category: 'Getting Started' },
  { id: 'config-guide', title: 'Configuration', category: 'Getting Started' },
  { id: 'macro-overview', title: 'Macros Overview', category: 'Getting Started' },

  // 2. Core Concepts - Foundation before building
  { id: 'di-guide', title: 'Dependency Injection', category: 'Core Concepts' },
  { id: 'lifecycle-hooks', title: 'Lifecycle Hooks', category: 'Core Concepts' },

  // 3. Routing & Controllers - Learn to handle requests first
  { id: 'route-groups', title: 'Route Groups', category: 'Routing', file: 'route-groups-guide.md' },
  { id: 'route-constraints', title: 'Route Constraints', category: 'Routing', file: 'route-constraints-guide.md' },
  { id: 'request-extractors', title: 'Request Extractors', category: 'Routing' },
  { id: 'use-middleware', title: 'Middleware', category: 'Routing', file: 'use-middleware-guide.md' },
  { id: 'guards-interceptors', title: 'Guards & Interceptors', category: 'Routing' },
  { id: 'use-guard', title: 'Using Guards', category: 'Routing', file: 'use-guard-guide.md' },

  // 4. Request & Response - HTTP handling
  { id: 'http-status-errors', title: 'HTTP Errors', category: 'HTTP' },
  { id: 'request-timeouts', title: 'Timeouts', category: 'HTTP', file: 'request-timeouts-guide.md' },
  { id: 'streaming-responses', title: 'Streaming', category: 'HTTP', file: 'streaming-responses-guide.md' },
  { id: 'compression', title: 'Compression', category: 'HTTP' },
  { id: 'https-guide', title: 'HTTPS & TLS', category: 'HTTP' },
  { id: 'acme-certificates', title: 'SSL Certificates', category: 'HTTP' },

  // 5. Security - After understanding routing
  { id: 'auth-guide', title: 'Authentication', category: 'Security' },
  { id: 'oauth2-providers', title: 'OAuth2 & Social Login', category: 'Security', file: 'oauth2-providers-guide.md' },
  { id: 'session-guide', title: 'Sessions', category: 'Security' },
  { id: 'rate-limiting', title: 'Rate Limiting', category: 'Security', file: 'rate-limiting-guide.md' },
  { id: 'security-guide', title: 'Security Best Practices', category: 'Security' },
  { id: 'security-advanced', title: 'Advanced Security', category: 'Security', file: 'security-advanced-guide.md' },

  // 6. API Features - Building APIs
  { id: 'api-versioning', title: 'API Versioning', category: 'API Features', file: 'api-versioning-guide.md' },
  {
    id: 'pagination-filtering',
    title: 'Pagination & Filtering',
    category: 'API Features',
    file: 'pagination-filtering-guide.md',
  },
  {
    id: 'content-negotiation',
    title: 'Content Negotiation',
    category: 'API Features',
    file: 'content-negotiation-guide.md',
  },
  { id: 'response-caching', title: 'HTTP Caching', category: 'API Features', file: 'response-caching-guide.md' },
  { id: 'etag-conditional', title: 'ETags', category: 'API Features', file: 'etag-conditional-requests-guide.md' },
  { id: 'openapi-guide', title: 'OpenAPI/Swagger', category: 'API Features' },

  // 7. Data & Caching - Persistence layer
  {
    id: 'cache-improvements',
    title: 'Caching Strategies',
    category: 'Data & Caching',
    file: 'cache-improvements-guide.md',
  },
  { id: 'redis-guide', title: 'Redis', category: 'Data & Caching' },

  // 8. Background Processing - Async work
  { id: 'queue-guide', title: 'Job Queues', category: 'Background Jobs' },
  { id: 'cron-guide', title: 'Scheduled Tasks', category: 'Background Jobs' },
  {
    id: 'graceful-shutdown',
    title: 'Graceful Shutdown',
    category: 'Background Jobs',
    file: 'graceful-shutdown-guide.md',
  },

  // 9. Real-Time - Live communication
  { id: 'websocket-sse', title: 'WebSockets & SSE', category: 'Real-Time', file: 'websocket-sse-guide.md' },
  { id: 'webhooks', title: 'Webhooks', category: 'Real-Time' },

  // 10. GraphQL - Alternative API paradigm
  { id: 'graphql-guide', title: 'GraphQL', category: 'GraphQL' },
  { id: 'graphql-config', title: 'GraphQL Config', category: 'GraphQL', file: 'graphql-configuration.md' },

  // 11. Cloud & Deployment - Production readiness
  { id: 'cloud-providers', title: 'Cloud SDKs', category: 'Cloud & Deployment', file: 'cloud-providers-guide.md' },
  { id: 'deployment-guide', title: 'Deployment Guide', category: 'Cloud & Deployment' },
  { id: 'docker-guide', title: 'Docker', category: 'Cloud & Deployment' },
  { id: 'kubernetes-guide', title: 'Kubernetes', category: 'Cloud & Deployment' },
  { id: 'scaling-guide', title: 'Scaling', category: 'Cloud & Deployment' },

  // 12. Observability - Monitoring and debugging
  { id: 'logging-guide', title: 'Logging', category: 'Observability' },
  { id: 'debug-logging', title: 'Debug Logging', category: 'Observability', file: 'debug-logging-guide.md' },
  { id: 'health-check', title: 'Health Checks', category: 'Observability', file: 'health-check-guide.md' },
  { id: 'metrics-guide', title: 'Metrics', category: 'Observability' },
  { id: 'grafana-dashboards', title: 'Grafana Dashboards', category: 'Observability' },
  { id: 'opentelemetry-guide', title: 'OpenTelemetry', category: 'Observability' },
  { id: 'error-correlation', title: 'Error Tracking', category: 'Observability', file: 'error-correlation-guide.md' },
  { id: 'audit-guide', title: 'Audit Logging', category: 'Observability' },

  // 13. Testing - Quality assurance
  { id: 'testing-guide', title: 'Testing', category: 'Testing' },
  { id: 'testing-coverage', title: 'Coverage', category: 'Testing' },
  { id: 'testing-documentation', title: 'Testing Best Practices', category: 'Testing' },

  // 14. Performance - Optimization tools
  {
    id: 'profiling-guide',
    title: 'CPU Profiling',
    category: 'Performance',
    file: null,
    hasComponent: true,
    description:
      'Profile an Armature service with perf, flamegraph and pprof: capture a CPU profile, read the flame graph, and find the hot path in a Rust web handler.',
  },

  // 15. Benchmarks - Performance comparisons
  { id: 'framework-comparison', title: 'vs Actix vs Axum', category: 'Benchmarks' },
  { id: 'armature-vs-nodejs', title: 'vs Node.js', category: 'Benchmarks', file: 'armature-vs-nodejs-benchmark.md' },
  { id: 'armature-vs-nextjs', title: 'vs Next.js', category: 'Benchmarks', file: 'armature-vs-nextjs-benchmark.md' },
];

/** Markdown filename in the repository docs/ directory, or null for component/placeholder pages. */
export function docFile(doc: DocMetadata): string | null {
  return doc.file === null ? null : (doc.file ?? `${doc.id}.md`);
}

export function getDoc(id: string): DocMetadata | undefined {
  return docs.find((d) => d.id === id);
}

/** Docs grouped by category, preserving first-appearance category order. */
export function docsByCategory(): Map<string, DocMetadata[]> {
  const grouped = new Map<string, DocMetadata[]>();
  for (const doc of docs) {
    const list = grouped.get(doc.category);
    if (list) {
      list.push(doc);
    } else {
      grouped.set(doc.category, [doc]);
    }
  }
  return grouped;
}
