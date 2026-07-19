/**
 * JSON-LD structured data, pre-serialized once at module load.
 * Organization and WebSite are site-wide; the rest are home-page-only
 * (they describe the product/FAQ content, not every subpage).
 */

export const SOFTWARE_APPLICATION_SCHEMA = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'SoftwareApplication',
  name: 'Armature Framework',
  alternateName: ['Armature', 'armature-rs'],
  applicationCategory: 'DeveloperApplication',
  applicationSubCategory: 'Web Framework',
  operatingSystem: 'Linux, macOS, Windows',
  programmingLanguage: {
    '@type': 'ComputerLanguage',
    name: 'Rust',
    url: 'https://www.rust-lang.org/',
  },
  description:
    'Armature is a batteries-included, enterprise-grade web framework for Rust inspired by NestJS and Angular. Features dependency injection, decorators, middleware, JWT/OAuth2/SAML authentication, validation, caching, job queues, GraphQL, and 150+ enterprise features.',
  url: 'https://quinnjr.github.io/armature/',
  softwareVersion: '0.1.0',
  releaseNotes: 'https://github.com/quinnjr/armature/releases',
  license: 'https://opensource.org/licenses/Apache-2.0',
  author: {
    '@type': 'Person',
    name: 'Joseph R. Quinn',
    email: 'pegasusheavyindustries@gmail.com',
    url: 'https://github.com/quinnjr',
  },
  publisher: {
    '@type': 'Organization',
    name: 'Pegasus Heavy Industries',
    url: 'https://github.com/quinnjr',
  },
  codeRepository: 'https://github.com/quinnjr/armature',
  downloadUrl: 'https://crates.io/crates/armature',
  installUrl: 'https://quinnjr.github.io/armature/getting-started',
  softwareHelp: 'https://quinnjr.github.io/armature/docs',
  programmingModel: 'Async/Await',
  runtimePlatform: 'Tokio',
  offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
  aggregateRating: {
    '@type': 'AggregateRating',
    ratingValue: '5',
    ratingCount: '1',
    bestRating: '5',
    worstRating: '1',
  },
  keywords:
    'rust, web framework, http server, nestjs alternative, express alternative, actix-web, rocket, axum, dependency injection, async rust, type-safe, middleware, jwt, oauth2, saml, graphql, openapi, swagger, enterprise, production-ready',
  featureList: [
    'Dependency Injection Container',
    'Decorator-based routing (#[controller], #[get], #[post])',
    'Module system for code organization',
    'Guards and Interceptors',
    'JWT Authentication',
    'OAuth2/OIDC (10+ providers)',
    'SAML 2.0 Enterprise SSO',
    'Two-Factor Authentication (TOTP/HOTP)',
    'Passwordless Auth (Magic Links, WebAuthn)',
    'API Key Management',
    'Rate Limiting (Token Bucket, Sliding Window)',
    'Data Validation Framework',
    'OpenAPI/Swagger Integration',
    'GraphQL Support',
    'WebSocket and Server-Sent Events',
    'Background Job Queues (Redis)',
    'Cron Scheduling',
    'Multi-tier Caching (Redis, Memcached)',
    'OpenTelemetry Observability',
    'Prometheus Metrics',
    'Health Checks (Kubernetes probes)',
    'AWS SDK Integration',
    'GCP SDK Integration',
    'Azure SDK Integration',
    'Serverless (Lambda, Cloud Run, Azure Functions)',
    'File Storage (S3, GCS, Azure Blob)',
    'Email (SMTP, SendGrid, SES, Mailgun)',
    'Push Notifications (Web Push, FCM, APNS)',
    'Security Headers (CSP, HSTS, CORS)',
  ],
  sameAs: ['https://github.com/quinnjr/armature', 'https://crates.io/crates/armature'],
});

export const ORGANIZATION_SCHEMA = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'Organization',
  name: 'Pegasus Heavy Industries',
  url: 'https://github.com/quinnjr',
  logo: 'https://quinnjr.github.io/armature/assets/armature-logo.svg',
  sameAs: ['https://github.com/quinnjr'],
});

export const WEBSITE_SCHEMA = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'WebSite',
  name: 'Armature Framework',
  url: 'https://quinnjr.github.io/armature/',
  potentialAction: {
    '@type': 'SearchAction',
    target: 'https://quinnjr.github.io/armature/docs?q={search_term_string}',
    'query-input': 'required name=search_term_string',
  },
});

export const FAQ_PAGE_SCHEMA = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'FAQPage',
  mainEntity: [
    {
      '@type': 'Question',
      name: 'Is Armature a good NestJS alternative for Rust?',
      acceptedAnswer: {
        '@type': 'Answer',
        text: "Yes! Armature is specifically designed to bring NestJS's architectural patterns to Rust. It features dependency injection, decorators (via Rust macros), modules, guards, interceptors, and pipes - all concepts familiar to NestJS developers.",
      },
    },
    {
      '@type': 'Question',
      name: 'How does Armature compare to Express or Koa?',
      acceptedAnswer: {
        '@type': 'Answer',
        text: "Armature provides Express's simplicity and Koa's async/await patterns, but adds type safety, dependency injection, and compile-time error catching. The middleware system will feel familiar, but you get Rust's performance (8-15x faster) and memory safety guarantees.",
      },
    },
    {
      '@type': 'Question',
      name: 'Why choose Armature over Actix-web, Rocket, or Axum?',
      acceptedAnswer: {
        '@type': 'Answer',
        text: 'Armature focuses on enterprise features and developer experience. While Actix-web, Rocket, and Axum are excellent low-level frameworks, Armature provides built-in authentication (JWT, OAuth2, SAML), validation, caching, job queues, and OpenAPI generation - reducing the need for external crates.',
      },
    },
    {
      '@type': 'Question',
      name: 'How fast is Armature compared to Node.js?',
      acceptedAnswer: {
        '@type': 'Answer',
        text: 'Armature is 8-15x faster than Node.js frameworks like Express and NestJS, with ~3x less memory usage and sub-2ms p99 latency for typical API endpoints.',
      },
    },
  ],
});

export const BREADCRUMB_SCHEMA = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'BreadcrumbList',
  itemListElement: [
    {
      '@type': 'ListItem',
      position: 1,
      name: 'Home',
      item: 'https://quinnjr.github.io/armature/',
    },
    {
      '@type': 'ListItem',
      position: 2,
      name: 'Documentation',
      item: 'https://quinnjr.github.io/armature/docs',
    },
    {
      '@type': 'ListItem',
      position: 3,
      name: 'Getting Started',
      item: 'https://quinnjr.github.io/armature/getting-started',
    },
  ],
});
