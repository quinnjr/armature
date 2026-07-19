import { readFileSync } from 'node:fs';
import { posix } from 'node:path';
import { resolve } from 'node:path';
import { marked } from 'marked';
import { docs, docFile } from './docs';
import { url } from './url';

/**
 * Repository docs/ directory. The site builds from web/, so the markdown
 * sources live one level up.
 */
export const DOCS_DIR = resolve(process.cwd(), '..', 'docs');

/** Reverse lookup: markdown filename (basename) -> doc id. */
const idByFile = new Map<string, string>();
for (const doc of docs) {
  const file = docFile(doc);
  if (file !== null) {
    idByFile.set(file.toLowerCase(), doc.id);
  }
}

/**
 * Rewrite intra-doc markdown links. The sources link to sibling `.md` files
 * (`[Config](config-guide.md)`, `[README](../README.md)`), which would 404
 * when resolved against the rendered page URLs. Links whose target is a
 * registered doc become site routes; other repo files fall back to GitHub.
 */
export function rewriteDocLink(href: string): string {
  if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith('#') || href.startsWith('/')) {
    return href;
  }
  const [pathPart, hash] = href.split('#', 2);
  if (!pathPart.toLowerCase().endsWith('.md')) {
    return href;
  }
  const suffix = hash ? `#${hash}` : '';
  // Resolve relative to the docs/ directory the source file lives in.
  const repoPath = posix.normalize(posix.join('docs', pathPart));
  if (repoPath.startsWith('docs/') && !repoPath.slice('docs/'.length).includes('/')) {
    const id = idByFile.get(repoPath.slice('docs/'.length).toLowerCase());
    if (id) {
      return `${url(`/docs/${id}`)}${suffix}`;
    }
  }
  // Not a registered doc (or outside docs/) — link to the file on GitHub.
  return `https://github.com/quinnjr/armature/blob/main/${repoPath}${suffix}`;
}

marked.use({
  walkTokens(token) {
    if (token.type === 'link') {
      token.href = rewriteDocLink(token.href);
    }
  },
});

/** Read a markdown file from the repository docs/ directory and render it to HTML. */
export function renderDocMarkdown(file: string): string {
  let markdown: string;
  try {
    markdown = readFileSync(resolve(DOCS_DIR, file), 'utf-8');
  } catch (e) {
    throw new Error(`failed to read markdown source "${file}": ${(e as Error).message}`);
  }
  if (!markdown.trim()) {
    throw new Error(`empty markdown source: ${file}`);
  }
  return marked.parse(markdown, { async: false });
}
