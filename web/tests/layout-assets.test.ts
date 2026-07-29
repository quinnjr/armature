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

function publicPathFor(reference: string): string {
  // Either an absolute URL (og:image/twitter:image) or a base-relative path
  // already extracted from a `url('/foo')` call (favicon, icons, manifest).
  const pathname = reference.startsWith('http') ? new URL(reference).pathname : reference;
  const withoutBase = pathname.startsWith(GH_PAGES_BASE) ? pathname.slice(GH_PAGES_BASE.length) : pathname;
  return resolve(PUBLIC_DIR, withoutBase.replace(/^\/+/, ''));
}

describe('BaseLayout static asset references', () => {
  it('og:image and twitter:image point at a file that exists in public/', () => {
    const ogImage = layoutSource.match(/property="og:image"\s+content="([^"]+)"/);
    const twitterImage = layoutSource.match(/name="twitter:image"\s+content="([^"]+)"/);

    expect(ogImage, 'og:image meta tag not found in BaseLayout.astro').not.toBeNull();
    expect(twitterImage, 'twitter:image meta tag not found in BaseLayout.astro').not.toBeNull();

    const ogPath = publicPathFor(ogImage![1]);
    const twitterPath = publicPathFor(twitterImage![1]);

    expect(existsSync(ogPath), `og:image asset missing: ${ogImage![1]} -> ${ogPath}`).toBe(true);
    expect(existsSync(twitterPath), `twitter:image asset missing: ${twitterImage![1]} -> ${twitterPath}`).toBe(true);
  });

  it('og:image is a 1200x630 PNG matching the declared og:image:width/height', () => {
    const ogImage = layoutSource.match(/property="og:image"\s+content="([^"]+)"/);
    const width = layoutSource.match(/property="og:image:width"\s+content="(\d+)"/);
    const height = layoutSource.match(/property="og:image:height"\s+content="(\d+)"/);
    expect(width).not.toBeNull();
    expect(height).not.toBeNull();

    const ogPath = publicPathFor(ogImage![1]);
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
      const path = publicPathFor(ref);
      expect(existsSync(path), `asset missing: ${ref} -> ${path}`).toBe(true);
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
