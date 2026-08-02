/**
 * `/llms-full.txt` — every guide concatenated as plain markdown.
 *
 * The companion to `llms.txt`: that file is an index an agent must then crawl,
 * this one is the whole corpus in a single request. Answer engines that support
 * it cite pages far more accurately when they have the prose rather than a link
 * list, and the crawl budget for ~60 pages collapses to one fetch.
 *
 * Served as markdown rather than rendered HTML on purpose — it is what the
 * consumer wants to read, and it costs no boilerplate to parse around.
 */
import type { APIRoute } from 'astro';
import { docs, docFile } from '../lib/docs';
import { docPlainText, lastModified, docsLastModified } from '../lib/docmeta';
import { absolute, REPO_URL } from '../lib/seo';
import { FRAMEWORK_VERSION, MSRV } from '../lib/version';

export const GET: APIRoute = ({ site }) => {
  const abs = (path: string) => absolute(path, site ?? undefined);
  const updated = docsLastModified();

  const parts: string[] = [
    '# Armature Framework — Full Documentation',
    '',
    '> Every Armature guide, concatenated as plain markdown for ingestion in a',
    '> single request. Each section names its canonical URL so a citation can',
    '> point at the page a human should read.',
    '',
    `Version: ${FRAMEWORK_VERSION} (pre-1.0)`,
    `MSRV: ${MSRV}`,
    'License: Apache-2.0',
    `Repository: ${REPO_URL}`,
    `Index: ${abs('/llms.txt')}`,
    ...(updated ? [`Last updated: ${updated.slice(0, 10)}`] : []),
    '',
    '---',
    '',
  ];

  for (const doc of docs) {
    const markdown = docPlainText(doc);
    if (markdown === null) continue;

    const path = doc.id === 'readme' ? '/docs' : `/docs/${doc.id}`;
    const modified = lastModified(docFile(doc));

    parts.push(
      `## ${doc.title}`,
      '',
      `Category: ${doc.category}`,
      `URL: ${abs(path)}`,
      ...(modified ? [`Updated: ${modified.slice(0, 10)}`] : []),
      '',
      markdown.trim(),
      '',
      '---',
      '',
    );
  }

  return new Response(parts.join('\n'), {
    headers: {
      'Content-Type': 'text/markdown; charset=utf-8',
      'Cache-Control': 'public, max-age=3600',
    },
  });
};
