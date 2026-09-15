/**
 * `/llms.txt` — the AI-readable site index, per llmstxt.org.
 *
 * Generated from the doc registry rather than hand-maintained. The static file
 * this replaces had drifted: it advertised version 0.1.0 and "98% Feature
 * Complete", and its link list was a snapshot that no longer matched the pages
 * the site actually builds. An answer engine quotes this file verbatim, so a
 * stale one is worse than none.
 */
import type { APIRoute } from 'astro';
import { docsByCategory } from '../lib/docs';
import { docDescription, docsLastModified } from '../lib/docmeta';
import { absolute, REPO_URL } from '../lib/seo';
import { FRAMEWORK_VERSION, MSRV, RUST_EDITION } from '../lib/version';

export const GET: APIRoute = ({ site }) => {
  const abs = (path: string) => absolute(path, site ?? undefined);
  const updated = docsLastModified();

  const lines: string[] = [
    '# Armature Framework',
    '',
    '> Armature is a batteries-included web framework for Rust that brings the',
    '> architecture of NestJS and Angular to the Rust ecosystem: dependency',
    '> injection, decorator macros, modules, guards, interceptors and pipes, plus',
    '> built-in authentication, validation, caching, job queues and GraphQL.',
    '',
    `Version: ${FRAMEWORK_VERSION} (pre-1.0; the public API can change between minor versions)`,
    `Language: Rust ${RUST_EDITION} edition, MSRV ${MSRV}`,
    'Runtime: Tokio + Hyper, async throughout',
    'License: Apache-2.0',
    `Repository: ${REPO_URL}`,
    `Crate: https://crates.io/crates/armature`,
    `API reference: https://docs.rs/armature`,
    ...(updated ? [`Documentation last updated: ${updated.slice(0, 10)}`] : []),
    '',
    '## What Armature is for',
    '',
    'Building production HTTP APIs and microservices in Rust when you want the',
    'structure of an opinionated framework — DI container, module graph,',
    'declarative routing, guards for authorization — rather than assembling those',
    'yourself on top of a lower-level library. It is a peer of Actix-web, Axum and',
    'Rocket at the HTTP layer, and a peer of NestJS in the shape of its',
    'application architecture.',
    '',
    '## Key facts',
    '',
    '- Routing is declarative via proc macros: `#[controller]`, `#[get]`, `#[post]`,',
    '  `#[put]`, `#[delete]`, `#[patch]`, `#[options]`, `#[head]`, `#[query]`.',
    '- The HTTP QUERY method (a safe method that carries a request body) is',
    '  supported end to end, including body-keyed response caching.',
    '- Dependency injection is field-based: declare a service type as a struct',
    '  field and it is injected; services are singletons shared via `Arc`.',
    '- Guards fail closed. A role or permission guard requires a verified user',
    '  context attached by an authentication layer; it denies when none is present.',
    '- The workspace is 60+ crates, feature-gated. `full` enables everything except',
    '  SAML; `full-with-saml` additionally enables SAML (needs libxmlsec1).',
    '- TLS is rustls-only; no OpenSSL/native-tls dependency.',
    '',
    '## Documentation',
    '',
  ];

  for (const [category, entries] of docsByCategory()) {
    lines.push(`### ${category}`, '');
    for (const doc of entries) {
      const path = doc.id === 'readme' ? '/docs' : `/docs/${doc.id}`;
      lines.push(`- [${doc.title}](${abs(path)}): ${docDescription(doc)}`);
    }
    lines.push('');
  }

  lines.push(
    '## Site pages',
    '',
    `- [Home](${abs('/')}): what Armature is, at a glance.`,
    `- [Getting Started](${abs('/getting-started')}): install, scaffold and run a first API.`,
    `- [Comparisons](${abs('/comparisons')}): Armature against Actix-web, Axum, Rocket, NestJS and Express.`,
    `- [Roadmap](${abs('/roadmap')}): shipped, in progress and planned.`,
    `- [FAQ](${abs('/faq')}): common questions, answered.`,
    '',
    '## Optional',
    '',
    `- [Full documentation text](${abs('/llms-full.txt')}): every guide concatenated as plain markdown, for ingestion in one request.`,
    `- [Releases](${REPO_URL}/releases): per-crate changelogs live in each crate's CHANGELOG.md.`,
    '',
  );

  return new Response(lines.join('\n'), {
    headers: {
      'Content-Type': 'text/plain; charset=utf-8',
      'Cache-Control': 'public, max-age=3600',
    },
  });
};
