/**
 * Site-wide JSON-LD, pre-serialized once at module load.
 *
 * Only the schemas that describe the *site* live here. Page-specific documents
 * (TechArticle, BreadcrumbList, FAQPage, CollectionPage) are built per page in
 * `seo.ts` — emitting one static breadcrumb and one static FAQ on every page
 * described neither the page it appeared on nor, in the FAQ's case, the
 * questions actually rendered.
 */
import { FRAMEWORK_VERSION, MSRV, RUST_EDITION } from './version';

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
  softwareVersion: FRAMEWORK_VERSION,
  releaseNotes: 'https://github.com/quinnjr/armature/releases',
  license: 'https://opensource.org/licenses/Apache-2.0',
  author: {
    '@type': 'Person',
    name: 'Joseph R. Quinn',
    email: 'quinn.josephr@proton.me',
    url: 'https://github.com/quinnjr',
  },
  publisher: {
    '@type': 'Organization',
    name: 'Joseph Quinn',
    url: 'https://github.com/quinnjr',
  },
  codeRepository: 'https://github.com/quinnjr/armature',
  downloadUrl: 'https://crates.io/crates/armature',
  installUrl: 'https://quinnjr.github.io/armature/getting-started',
  softwareHelp: 'https://quinnjr.github.io/armature/docs',
  programmingModel: 'Async/Await',
  runtimePlatform: `Tokio, Rust ${RUST_EDITION} edition (MSRV ${MSRV})`,
  offers: { '@type': 'Offer', price: '0', priceCurrency: 'USD' },
  // No `aggregateRating`: the previous 5-stars-from-1-rating was self-issued.
  // Google's structured-data policy forbids self-serving review markup for the
  // entity publishing it, and enforcement is a manual action against the whole
  // site — a rich-result gamble with the site's indexing as the stake.
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
  name: 'Joseph Quinn',
  url: 'https://github.com/quinnjr',
  logo: 'https://quinnjr.github.io/armature/assets/armature-logo.svg',
  sameAs: ['https://github.com/quinnjr'],
});

export const WEBSITE_SCHEMA = JSON.stringify({
  '@context': 'https://schema.org',
  '@type': 'WebSite',
  name: 'Armature Framework',
  url: 'https://quinnjr.github.io/armature/',
  // No `SearchAction`: the sitelinks searchbox requires a working search
  // endpoint, and `/docs?q=` is not one — the docs index ignores the parameter.
  // Declaring the action anyway asks Google to send users to a URL that does
  // nothing with their query.
  inLanguage: 'en',
});
