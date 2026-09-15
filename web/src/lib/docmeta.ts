/**
 * Build-time metadata for documentation pages: a unique meta description and a
 * real last-modified date for each one.
 *
 * Both are derived rather than hand-maintained. Descriptions come from the
 * markdown itself (see `excerpt.ts`) so a new doc cannot ship sharing the
 * site-wide default with 60 other pages, and dates come from git so
 * `dateModified` reflects when the content actually changed — a checkout's file
 * mtimes are all "now", which would claim every page was updated today.
 */
import { execFileSync } from 'node:child_process';
import { statSync } from 'node:fs';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { docs, docFile, type DocMetadata } from './docs';
import { DOCS_DIR } from './markdown';
import { excerptFromMarkdown } from './excerpt';

/** Repository root: the site builds from `web/`, the docs live one level up. */
const REPO_ROOT = resolve(process.cwd(), '..');

/**
 * Last commit date per `docs/` file, ISO-8601, from a single `git log` pass.
 *
 * One subprocess for the whole tree rather than one per file. `--name-only`
 * pairs each commit date with the files it touched; walking newest-first and
 * keeping the first sighting of each path gives its most recent change.
 */
function gitLastModified(): Map<string, string> {
  const dates = new Map<string, string>();
  let out: string;
  try {
    out = execFileSync('git', ['log', '--pretty=format:%x00%cI', '--name-only', '--', 'docs/'], {
      cwd: REPO_ROOT,
      encoding: 'utf-8',
      maxBuffer: 32 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'ignore'],
    });
  } catch {
    // Not a git checkout (a published tarball, a shallow export). Callers fall
    // back to file mtimes; a missing date is better than a wrong one.
    return dates;
  }

  let current: string | null = null;
  for (const line of out.split('\n')) {
    if (line.startsWith('\0')) {
      current = line.slice(1).trim();
      continue;
    }
    const path = line.trim();
    // First sighting wins: git log is newest-first.
    if (current && path.startsWith('docs/') && !dates.has(path)) {
      dates.set(path, current);
    }
  }
  return dates;
}

const GIT_DATES = gitLastModified();

/** ISO-8601 last-modified for a `docs/` markdown file, or `null` if unknown. */
export function lastModified(file: string | null): string | null {
  if (file === null) return null;
  const fromGit = GIT_DATES.get(`docs/${file}`);
  if (fromGit) return fromGit;
  try {
    return statSync(resolve(DOCS_DIR, file)).mtime.toISOString();
  } catch {
    return null;
  }
}

/** Newest last-modified across every doc — the freshness of the docs section. */
export function docsLastModified(): string | null {
  let newest: string | null = null;
  for (const doc of docs) {
    const date = lastModified(docFile(doc));
    if (date && (newest === null || date > newest)) {
      newest = date;
    }
  }
  return newest;
}

/**
 * Meta description for a doc page.
 *
 * Prefers an explicit `description` in the registry, then the first prose
 * paragraph of the markdown, then a generated fallback naming the page and its
 * category — which is still page-specific, unlike the site-wide default.
 */
export function docDescription(doc: DocMetadata): string {
  if (doc.description) return doc.description;

  const file = docFile(doc);
  if (file !== null) {
    try {
      const excerpt = excerptFromMarkdown(readFileSync(resolve(DOCS_DIR, file), 'utf-8'));
      if (excerpt) return excerpt;
    } catch {
      // Fall through to the generated description.
    }
  }
  return `${doc.title} — ${doc.category} documentation for Armature, the batteries-included Rust web framework with NestJS-style dependency injection and decorators.`;
}

/** Plain-text body of a doc, for the AI-oriented `llms-full.txt` dump. */
export function docPlainText(doc: DocMetadata): string | null {
  const file = docFile(doc);
  if (file === null) return null;
  try {
    return readFileSync(resolve(DOCS_DIR, file), 'utf-8');
  } catch {
    return null;
  }
}
