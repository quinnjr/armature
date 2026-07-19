export interface DocLink {
  id: string;
  title: string;
  description: string;
}

export interface DocCategory {
  name: string;
  docs: DocLink[];
}

// Documentation index shown on the docs overview page, grouped by topic.
export const categories: DocCategory[] = [
  {
    name: 'Core Guides',
    docs: [
      {
        id: 'di-guide',
        title: 'Dependency Injection',
        description: 'Service injection, module system, best practices',
      },
      {
        id: 'config-guide',
        title: 'Configuration',
        description: 'Environment variables, type-safe config, validation',
      },
      { id: 'lifecycle-hooks', title: 'Lifecycle Hooks', description: 'OnInit, OnDestroy, module lifecycle' },
      { id: 'project-templates', title: 'Project Templates', description: 'Starter templates and scaffolding' },
      { id: 'macro-overview', title: 'Macros Overview', description: 'Decorator macros and code generation' },
    ],
  },
  {
    name: 'Authentication & Security',
    docs: [
      { id: 'auth-guide', title: 'Authentication', description: 'Password hashing, JWT, guards, RBAC' },
      {
        id: 'oauth2-providers',
        title: 'OAuth2 Providers',
        description: 'Google, Microsoft, GitHub, Discord, and more',
      },
      { id: 'security-guide', title: 'Security Best Practices', description: 'CORS, CSP, HSTS, request signing' },
      { id: 'security-advanced', title: 'Advanced Security', description: '2FA, WebAuthn, API keys' },
      { id: 'session-guide', title: 'Session Management', description: 'Redis-backed sessions, cookies' },
      { id: 'rate-limiting', title: 'Rate Limiting', description: 'Token bucket, sliding window algorithms' },
    ],
  },
  {
    name: 'Routing & Controllers',
    docs: [
      { id: 'route-groups', title: 'Route Groups', description: 'Organizing routes with shared middleware' },
      { id: 'route-constraints', title: 'Route Constraints', description: 'Parameter validation at route level' },
      { id: 'guards-interceptors', title: 'Guards & Interceptors', description: 'Cross-cutting concerns' },
      { id: 'use-guard', title: 'Using Guards', description: 'Authorization and access control' },
      { id: 'use-middleware', title: 'Using Middleware', description: 'Request/response middleware' },
      { id: 'request-extractors', title: 'Request Extractors', description: 'Body, Query, Path, Header extractors' },
    ],
  },
  {
    name: 'API Features',
    docs: [
      { id: 'api-versioning', title: 'API Versioning', description: 'URL, header, and query-based versioning' },
      { id: 'content-negotiation', title: 'Content Negotiation', description: 'Accept header handling' },
      { id: 'pagination-filtering', title: 'Pagination & Filtering', description: 'Offset/cursor pagination, sorting' },
      { id: 'response-caching', title: 'Response Caching', description: 'Cache-Control, ETags' },
      { id: 'etag-conditional', title: 'ETags & Conditional Requests', description: 'If-Match, If-None-Match' },
    ],
  },
  {
    name: 'Real-Time & Networking',
    docs: [
      { id: 'websocket-sse', title: 'WebSockets & SSE', description: 'Real-time bidirectional communication' },
      { id: 'webhooks', title: 'Webhooks', description: 'Webhook sending and receiving' },
      { id: 'https-guide', title: 'HTTPS & TLS', description: 'TLS configuration' },
      { id: 'streaming-responses', title: 'Streaming Responses', description: 'Chunked transfer, large files' },
      { id: 'request-timeouts', title: 'Request Timeouts', description: 'Configurable timeouts' },
    ],
  },
  {
    name: 'GraphQL & OpenAPI',
    docs: [
      { id: 'graphql-guide', title: 'GraphQL Guide', description: 'Schema-first and code-first GraphQL' },
      { id: 'graphql-config', title: 'GraphQL Configuration', description: 'Advanced GraphQL options' },
      { id: 'openapi-guide', title: 'OpenAPI/Swagger', description: 'Auto-generated API documentation' },
    ],
  },
  {
    name: 'Background Processing',
    docs: [
      { id: 'queue-guide', title: 'Job Queues', description: 'Redis-backed background jobs' },
      { id: 'cron-guide', title: 'Cron Jobs', description: 'Scheduled tasks' },
      { id: 'graceful-shutdown', title: 'Graceful Shutdown', description: 'Connection draining, cleanup hooks' },
    ],
  },
  {
    name: 'Caching & Data',
    docs: [
      { id: 'cache-improvements', title: 'Caching Strategies', description: 'Multi-tier caching, tag invalidation' },
      { id: 'redis-guide', title: 'Redis Integration', description: 'Centralized Redis client' },
    ],
  },
  {
    name: 'Cloud Providers',
    docs: [{ id: 'cloud-providers', title: 'Cloud SDKs', description: 'AWS, GCP, Azure integration' }],
  },
  {
    name: 'Observability',
    docs: [
      { id: 'logging-guide', title: 'Structured Logging', description: 'JSON logging with tracing' },
      { id: 'opentelemetry-guide', title: 'OpenTelemetry', description: 'Distributed tracing and metrics' },
      { id: 'metrics-guide', title: 'Prometheus Metrics', description: 'Custom metrics, /metrics endpoint' },
      { id: 'health-check', title: 'Health Checks', description: 'Liveness, readiness, startup probes' },
      { id: 'audit-guide', title: 'Audit Logging', description: 'Who did what, when' },
      { id: 'error-correlation', title: 'Error Correlation', description: 'Request ID tracking' },
    ],
  },
  {
    name: 'Testing',
    docs: [
      { id: 'testing-guide', title: 'Testing Guide', description: 'Unit, integration, e2e testing' },
      { id: 'testing-coverage', title: 'Test Coverage', description: 'Coverage reporting' },
      { id: 'testing-documentation', title: 'Testing Best Practices', description: 'Testing patterns' },
    ],
  },
  {
    name: 'Benchmarks',
    docs: [
      { id: 'armature-vs-nodejs', title: 'vs Node.js', description: 'Performance comparison with Express, NestJS' },
      { id: 'armature-vs-nextjs', title: 'vs Next.js', description: 'Performance comparison with Next.js' },
    ],
  },
];

// Cloud provider SDK crates shown in the overview table.
export const cloudProviders = [
  {
    name: 'armature-aws',
    provider: 'Amazon Web Services',
    services: 'S3, DynamoDB, SQS, SNS, SES, Lambda, KMS, Cognito',
  },
  {
    name: 'armature-gcp',
    provider: 'Google Cloud Platform',
    services: 'Storage, Pub/Sub, Firestore, Spanner, BigQuery',
  },
  { name: 'armature-azure', provider: 'Microsoft Azure', services: 'Blob, Queue, Cosmos, Service Bus, Key Vault' },
];

// Example programs in the repository's examples/ directory.
export const examples = [
  { name: 'crud_api.rs', description: 'Complete REST API with CRUD operations' },
  { name: 'auth_api.rs', description: 'JWT authentication flow' },
  { name: 'realtime_api.rs', description: 'WebSocket/SSE real-time communication' },
  { name: 'dependency_injection.rs', description: 'DI patterns and best practices' },
  { name: 'websocket_chat.rs', description: 'WebSocket chat room' },
];
