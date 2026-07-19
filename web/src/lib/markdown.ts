import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { marked } from 'marked';

/**
 * Repository docs/ directory. The site builds from web/, so the markdown
 * sources live one level up.
 */
export const DOCS_DIR = resolve(process.cwd(), '..', 'docs');

/** Read a markdown file from the repository docs/ directory and render it to HTML. */
export function renderDocMarkdown(file: string): string {
  const markdown = readFileSync(resolve(DOCS_DIR, file), 'utf-8');
  return marked.parse(markdown, { async: false });
}
