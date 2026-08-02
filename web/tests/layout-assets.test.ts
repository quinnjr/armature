import { existsSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Regression test for: og:image/twitter:image (and friends) pointing at a static
// asset that doesn't actually exist under web/public/, which 404s on every page
// load and breaks link unfurls on Twitter/Slack/Discord/Facebook.
//
// BaseLayout.astro is not compiled here (vitest runs in a plain node
// environment, see vitest.config.ts), so this reads it as text and extracts
// every static asset reference from its <meta>/<link> tags, then asserts the
// referenced file exists on disk under web/public/.

const PUBLIC_DIR = resolve(process.cwd(), 'public');
const LAYOUT_PATH = resolve(process.cwd(), 'src/layouts/BaseLayout.astro');
const layoutSource = readFileSync(LAYOUT_PATH, 'utf-8');

// The base path Astro is built with for GitHub Pages (see astro.config.mjs,
// which reads `base: process.env.ASTRO_BASE ?? '/'`). Absolute meta-tag URLs
// (og:image, twitter:image) are written against this deployed base rather
// than through the `url()` helper, so it must be stripped to recover the
// path relative to web/public/.
//
// Rather than hardcoding the value a second time (which could silently drift
// from package.json's `build:gh-pages` script), derive it from that script
// directly so the two stay in sync.
function getGhPagesBase(): string {
  const pkgJsonPath = resolve(process.cwd(), 'package.json');
  const pkg = JSON.parse(readFileSync(pkgJsonPath, 'utf-8')) as {
    scripts?: Record<string, string>;
  };
  const script = pkg.scripts?.['build:gh-pages'] ?? '';
  const match = script.match(/ASTRO_BASE=(\S+)/);
  if (!match) {
    throw new Error(
      "Could not find ASTRO_BASE in package.json's build:gh-pages script — update GH_PAGES_BASE derivation logic",
    );
  }
  return match[1];
}

const GH_PAGES_BASE = getGhPagesBase();

/**
 * Routes emitted by a page module rather than shipped in `public/`.
 * `llms.txt` is generated from the documentation registry, so asserting it
 * exists on disk would fail even though the deployed site serves it.
 */
const GENERATED_ROUTES = new Set(['/llms.txt', '/llms-full.txt']);

function publicPathFor(reference: string): string {
  // Either an absolute URL (og:image/twitter:image) or a base-relative path
  // already extracted from a `url('/foo')` call (favicon, icons, manifest).
  const pathname = reference.startsWith('http') ? new URL(reference).pathname : reference;
  const withoutBase = pathname.startsWith(GH_PAGES_BASE) ? pathname.slice(GH_PAGES_BASE.length) : pathname;
  return resolve(PUBLIC_DIR, withoutBase.replace(/^\/+/, ''));
}

/**
 * The social image is no longer a hardcoded absolute URL — it is built from a
 * site-relative path through `absolute()` so it follows `Astro.site` and the
 * configured base. The asset still has to exist, so recover the path from the
 * `absolute('/...')` call instead of from the emitted attribute.
 */
function socialImagePath(): string {
  const match = layoutSource.match(/const ogImage = absolute\('([^']+)'/);
  if (!match) {
    throw new Error("Could not find the og:image source in BaseLayout.astro — update socialImagePath()");
  }
  return match[1];
}

describe('BaseLayout static asset references', () => {
  it('og:image and twitter:image are both bound to an asset that exists in public/', () => {
    // Both tags must render the same derived value; a literal URL in either one
    // would silently stop tracking the deployed base.
    expect(layoutSource).toContain('<meta property="og:image" content={ogImage} />');
    expect(layoutSource).toContain('<meta name="twitter:image" content={ogImage} />');

    const path = publicPathFor(socialImagePath());
    expect(existsSync(path), `og:image asset missing: ${socialImagePath()} -> ${path}`).toBe(true);
  });

  it('og:image is a 1200x630 PNG matching the declared og:image:width/height', () => {
    const width = layoutSource.match(/property="og:image:width"\s+content="(\d+)"/);
    const height = layoutSource.match(/property="og:image:height"\s+content="(\d+)"/);
    expect(width).not.toBeNull();
    expect(height).not.toBeNull();

    const ogPath = publicPathFor(socialImagePath());
    const buf = readFileSync(ogPath);
    // PNG signature + IHDR chunk: width/height are big-endian u32s at offset 16/20.
    expect(buf.subarray(0, 8).toString('hex')).toBe('89504e470d0a1a0a');
    expect(buf.readUInt32BE(16)).toBe(Number(width![1]));
    expect(buf.readUInt32BE(20)).toBe(Number(height![1]));
  });

  it('favicon, apple-touch-icon, and manifest links point at files that exist in public/', () => {
    const urlCalls = [...layoutSource.matchAll(/href=\{url\('([^']+)'\)\}/g)].map((m) => m[1]);

    // Sanity check the extraction itself isn't silently matching nothing
    // (e.g. because BaseLayout.astro was refactored to build hrefs differently).
    expect(urlCalls.length).toBeGreaterThanOrEqual(4);
    expect(urlCalls).toContain('/favicon.ico');
    expect(urlCalls).toContain('/apple-touch-icon.png');
    expect(urlCalls).toContain('/manifest.webmanifest');

    for (const ref of urlCalls) {
      if (GENERATED_ROUTES.has(ref)) continue;
      const path = publicPathFor(ref);
      expect(existsSync(path), `asset missing: ${ref} -> ${path}`).toBe(true);
    }
  });

  it('every generated route the layout links to is backed by a page module', () => {
    const urlCalls = new Set([...layoutSource.matchAll(/href=\{url\('([^']+)'\)\}/g)].map((m) => m[1]));

    for (const route of GENERATED_ROUTES) {
      // Only assert the ones the layout actually references, so an unused entry
      // in the set cannot mask a deleted endpoint.
      if (!urlCalls.has(route)) continue;
      const page = resolve(process.cwd(), 'src/pages', `${route.replace(/^\/+/, '')}.ts`);
      expect(existsSync(page), `generated route ${route} has no page module at ${page}`).toBe(true);
    }
  });

  it('every icon declared in manifest.webmanifest exists in public/', () => {
    const manifestPath = resolve(PUBLIC_DIR, 'manifest.webmanifest');
    expect(existsSync(manifestPath)).toBe(true);

    const manifest = JSON.parse(readFileSync(manifestPath, 'utf-8')) as {
      icons?: Array<{ src: string }>;
    };
    expect(manifest.icons?.length ?? 0).toBeGreaterThan(0);

    for (const icon of manifest.icons ?? []) {
      const path = resolve(PUBLIC_DIR, icon.src);
      expect(existsSync(path), `manifest icon missing: ${icon.src} -> ${path}`).toBe(true);
    }
  });
});
