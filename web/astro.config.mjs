// @ts-check
import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';
import { execFileSync } from 'node:child_process';
// Vite transpiles the config, so the TypeScript registry is importable here and
// the sitemap cannot list a different set of docs than the site builds.
import { docs, docFile } from './src/lib/docs';

/**
 * Last commit date per `docs/` file, so sitemap `lastmod` reflects when the
 * content changed rather than when CI checked it out — a checkout stamps every
 * file with "now", which tells a crawler the whole site changed on every build
 * and teaches it to ignore the signal.
 */
function docDates() {
  const dates = new Map();
  try {
    const out = execFileSync('git', ['log', '--pretty=format:%x00%cI', '--name-only', '--', 'docs/'], {
      cwd: '..',
      encoding: 'utf-8',
      maxBuffer: 32 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'ignore'],
    });
    let current = null;
    for (const line of out.split('\n')) {
      if (line.startsWith('\0')) {
        current = line.slice(1).trim();
        continue;
      }
      const path = line.trim();
      if (current && path.startsWith('docs/') && !dates.has(path)) {
        dates.set(path, current);
      }
    }
  } catch {
    // Not a git checkout; omit lastmod rather than assert a wrong date.
  }
  return dates;
}

const DOC_DATES = docDates();
const NEWEST_DOC = [...DOC_DATES.values()].sort().at(-1);

/** `/docs/<id>` -> that guide's own last-commit date. */
const DATE_BY_PATH = new Map(
  docs.flatMap((doc) => {
    const file = docFile(doc);
    const date = file === null ? undefined : DOC_DATES.get(`docs/${file}`);
    if (!date) return [];
    return [[doc.id === 'readme' ? '/docs' : `/docs/${doc.id}`, date]];
  }),
);

// ASTRO_BASE is set to /armature for GitHub Pages builds (see build:gh-pages script).
// File output + no trailing slash preserves the pre-Astro URL contract
// (/armature/docs/auth-guide, exact, no redirect) on GitHub Pages.
export default defineConfig({
  site: 'https://quinnjr.github.io',
  base: process.env.ASTRO_BASE ?? '/',
  trailingSlash: 'never',
  build: {
    format: 'file',
  },
  integrations: [
    sitemap({
      // 404 has no business in a sitemap; the text endpoints are for agents,
      // which reach them from robots.txt and the <link rel="alternate">.
      filter: (page) => !/\/404(\.html)?$/.test(page) && !/llms(-full)?\.txt$/.test(page),
      serialize(item) {
        const path = new URL(item.url).pathname.replace(/\/armature/, '').replace(/\.html$/, '') || '/';

        // Priority mirrors how the site actually funnels readers: landing and
        // docs root first, then the guides, then the standing pages.
        if (path === '/') {
          item.priority = 1.0;
          item.changefreq = 'weekly';
        } else if (path === '/docs' || path === '/getting-started') {
          item.priority = 0.9;
          item.changefreq = 'weekly';
        } else if (path.startsWith('/docs/')) {
          item.priority = 0.8;
          item.changefreq = 'monthly';
        } else {
          item.priority = 0.6;
          item.changefreq = 'monthly';
        }

        // A guide's own commit date where we have one; otherwise the newest
        // change anywhere in the docs, which is the best honest answer for a
        // hand-written page whose content lives in this repo's history too.
        const lastmod = DATE_BY_PATH.get(path) ?? NEWEST_DOC;
        if (lastmod) {
          item.lastmod = lastmod;
        }
        return item;
      },
    }),
  ],
  vite: {
    plugins: [tailwindcss()],
  },
});
