/**
 * Meta descriptions derived from markdown sources.
 *
 * Every doc page previously inherited the site-wide default description, so
 * ~60 pages shipped byte-identical `<meta name="description">` and identical
 * social/AI previews. Search engines collapse duplicate snippets and answer
 * engines have nothing page-specific to quote. Deriving one from the source
 * means a page cannot ship without a description, and it cannot drift from the
 * content the way a hand-maintained table would.
 */

/** Google truncates around 155-160 characters; leave room for the ellipsis. */
const MAX_LENGTH = 155;

/**
 * Strip markdown to the plain prose a search engine would quote.
 *
 * Deliberately lossy and order-dependent: fenced code and HTML blocks go first
 * so their contents can never leak into a snippet, then inline constructs are
 * unwrapped to their visible text.
 */
function stripMarkdown(markdown: string): string {
  return (
    markdown
      // Fenced code, HTML comments, and raw HTML blocks: never quotable prose.
      .replace(/```[\s\S]*?```/g, ' ')
      .replace(/~~~[\s\S]*?~~~/g, ' ')
      .replace(/<!--[\s\S]*?-->/g, ' ')
      .replace(/<\/?[a-z][^>]*>/gi, ' ')
      // Images before links: an image's alt text is not a sentence.
      .replace(/!\[[^\]]*\]\([^)]*\)/g, ' ')
      .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
      .replace(/\[([^\]]*)\]\[[^\]]*\]/g, '$1')
      // Inline code and emphasis keep their text, lose their punctuation.
      .replace(/`([^`]*)`/g, '$1')
      .replace(/(\*\*|__)(.*?)\1/g, '$2')
      .replace(/(\*|_)(.*?)\1/g, '$2')
      .replace(/~~(.*?)~~/g, '$1')
      // Leading block markers. The indent class is `[ \t]`, not `\s`: `\s`
      // matches a newline, so `^\s{0,3}#{1,6} ` would consume the blank line
      // separating a paragraph from the heading below it and splice the two
      // together — which is how a table of contents ended up inside a meta
      // description.
      .replace(/^[ \t]{0,3}>[ \t]?/gm, '')
      .replace(/^[ \t]{0,3}#{1,6}[ \t]+/gm, '')
      .replace(/^[ \t]{0,3}[-*+][ \t]+/gm, '')
      .replace(/^[ \t]{0,3}\d+\.[ \t]+/gm, '')
      .replace(/^[ \t]{0,3}(?:[-*_][ \t]*){3,}$/gm, ' ')
      .replace(/[ \t]+/g, ' ')
      .trim()
  );
}

/**
 * Whether a stripped paragraph reads as a description rather than scaffolding.
 *
 * Badge rows, tables and one-word lines survive stripping but say nothing to a
 * reader; requiring a sentence-like length and a terminator keeps them out.
 */
function isProse(paragraph: string): boolean {
  if (paragraph.length < 40) return false;
  if (paragraph.startsWith('|') || paragraph.includes('|---')) return false;
  return /[.!?]/.test(paragraph) || paragraph.split(' ').length >= 8;
}

/** Cut to `MAX_LENGTH` on a word boundary, preferring a whole sentence. */
export function truncate(text: string, max: number = MAX_LENGTH): string {
  if (text.length <= max) return text;

  // A sentence that ends comfortably inside the budget beats a mid-clause cut.
  const sentenceEnd = text.slice(0, max + 1).search(/[.!?](?:\s|$)/);
  if (sentenceEnd >= Math.floor(max * 0.6)) {
    return text.slice(0, sentenceEnd + 1).trim();
  }

  const cut = text.lastIndexOf(' ', max - 1);
  return `${text.slice(0, cut > 0 ? cut : max - 1).trimEnd()}…`;
}

/**
 * The first prose paragraph of `markdown`, trimmed to a meta-description length.
 *
 * Returns `null` when the source has no prose worth quoting, so the caller can
 * fall back rather than emit a truncated table row.
 */
export function excerptFromMarkdown(markdown: string): string | null {
  const body = stripMarkdown(markdown);
  if (!body) return null;

  for (const paragraph of body.split(/\n\s*\n/)) {
    const line = paragraph.replace(/\s*\n\s*/g, ' ').trim();
    if (isProse(line)) {
      return truncate(line);
    }
  }
  return null;
}
