import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { docs, docFile, docsByCategory, getDoc } from '../src/lib/docs';
import { categories as overviewCategories } from '../src/lib/overview';
import { DOCS_DIR, renderDocMarkdown, renderMarkdown, rewriteDocLink } from '../src/lib/markdown';

describe('docs registry', () => {
  it('has unique ids', () => {
    const ids = docs.map((d) => d.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('every markdown-backed doc points to an existing file in docs/', () => {
    const missing = docs
      .map((doc) => ({ doc, file: docFile(doc) }))
      .filter(({ file }) => file !== null)
      .filter(({ file }) => !existsSync(resolve(DOCS_DIR, file!)))
      .map(({ doc, file }) => `${doc.id} -> ${file}`);
    expect(missing).toEqual([]);
  });

  it('has readme doc', () => {
    const readme = getDoc('readme');
    expect(readme).toBeDefined();
    expect(readme?.title).toBe('Overview');
    expect(readme?.category).toBe('Getting Started');
  });

  it('has authentication guide', () => {
    const auth = getDoc('auth-guide');
    expect(auth).toBeDefined();
    expect(auth?.category).toBe('Security');
  });

  it('groups docs by category preserving order', () => {
    const grouped = docsByCategory();
    const categories = [...grouped.keys()];
    expect(categories[0]).toBe('Getting Started');
    expect(categories).toContain('Security');
    expect(categories).toContain('Observability');
    // every doc is in exactly one group
    const total = [...grouped.values()].reduce((n, list) => n + list.length, 0);
    expect(total).toBe(docs.length);
  });

  it('defaults markdown file to <id>.md and honors explicit overrides', () => {
    expect(docFile({ id: 'auth-guide', title: '', category: '' })).toBe('auth-guide.md');
    expect(docFile({ id: 'readme', title: '', category: '', file: 'README.md' })).toBe('README.md');
    expect(docFile({ id: 'x', title: '', category: '', file: null })).toBeNull();
  });
});

describe('docs overview', () => {
  it('links only to docs that exist in the registry (no dead /docs routes)', () => {
    const registryIds = new Set(docs.map((d) => d.id));
    const dead = overviewCategories
      .flatMap((category) => category.docs.map((doc) => doc.id))
      .filter((id) => !registryIds.has(id));
    expect(dead).toEqual([]);
  });
});

describe('markdown rendering', () => {
  it('renders a documentation file to HTML', () => {
    const html = renderDocMarkdown('auth-guide.md');
    expect(html).toContain('<h1');
    expect(html.length).toBeGreaterThan(1000);
  });

  it('renders every markdown-backed doc to non-trivial HTML', () => {
    for (const doc of docs) {
      const file = docFile(doc);
      if (file !== null) {
        const html = renderDocMarkdown(file);
        expect(typeof html, `rendering ${file}`).toBe('string');
        expect(html.length, `rendering ${file}`).toBeGreaterThan(100);
        expect(html, `rendering ${file}`).not.toContain('[object Promise]');
      }
    }
  });

  it('rewrites intra-doc markdown links to site routes', () => {
    expect(rewriteDocLink('auth-guide.md')).toBe('/docs/auth-guide');
    expect(rewriteDocLink('./config-guide.md')).toBe('/docs/config-guide');
    expect(rewriteDocLink('README.md')).toBe('/docs/readme');
    expect(rewriteDocLink('di-guide.md#usage')).toBe('/docs/di-guide#usage');
  });

  it('sends non-docs markdown files to GitHub and leaves other links alone', () => {
    // ../README.md escapes docs/ — it is the repo README, not docs/README.md
    expect(rewriteDocLink('../README.md')).toBe('https://github.com/quinnjr/armature/blob/main/README.md');
    expect(rewriteDocLink('../armature-macros/README.md')).toBe(
      'https://github.com/quinnjr/armature/blob/main/armature-macros/README.md'
    );
    expect(rewriteDocLink('plans/some-plan.md')).toBe(
      'https://github.com/quinnjr/armature/blob/main/docs/plans/some-plan.md'
    );
    expect(rewriteDocLink('https://example.com/x.md')).toBe('https://example.com/x.md');
    expect(rewriteDocLink('#section')).toBe('#section');
    expect(rewriteDocLink('/docs/auth-guide')).toBe('/docs/auth-guide');
  });

  it('strips script and event-handler HTML from markdown', () => {
    const html = renderMarkdown(
      '# Title\n\n<script>alert(1)</script>\n\n<img src="x.png" onerror="alert(1)" alt="ok">\n\n[link](https://example.com)'
    );
    expect(html).not.toContain('<script');
    expect(html).not.toContain('onerror');
    expect(html).toContain('<h1');
    expect(html).toContain('<img');
    expect(html).toContain('alt="ok"');
  });

  it('keeps the markup marked emits (code classes, tables, task lists)', () => {
    const html = renderMarkdown(
      '```rust\nfn main() {}\n```\n\n| a | b |\n|:--|--:|\n| 1 | 2 |\n\n- [x] done\n'
    );
    expect(html).toContain('language-rust');
    expect(html).toContain('<table');
    expect(html).toContain('<input');
  });

  it('no rendered doc contains a relative .md href', () => {
    for (const doc of docs) {
      const file = docFile(doc);
      if (file === null) continue;
      const html = renderDocMarkdown(file);
      const badLinks = [...html.matchAll(/href="([^"]+\.md[^"]*)"/g)]
        .map((m) => m[1])
        .filter((href) => !href.startsWith('http'));
      expect(badLinks, `in ${file}`).toEqual([]);
    }
  });
});
