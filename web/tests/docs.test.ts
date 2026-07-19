import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { docs, docFile, docsByCategory, getDoc } from '../src/lib/docs';
import { DOCS_DIR, renderDocMarkdown } from '../src/lib/markdown';

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

describe('markdown rendering', () => {
  it('renders a documentation file to HTML', () => {
    const html = renderDocMarkdown('auth-guide.md');
    expect(html).toContain('<h1');
    expect(html.length).toBeGreaterThan(1000);
  });

  it('renders every markdown-backed doc without throwing', () => {
    for (const doc of docs) {
      const file = docFile(doc);
      if (file !== null) {
        expect(() => renderDocMarkdown(file), `failed rendering ${file}`).not.toThrow();
      }
    }
  });
});
