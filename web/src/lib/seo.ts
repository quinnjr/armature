/**
 * Site identity and per-page SEO metadata.
 *
 * Absolute URLs are built here rather than hardcoded so the canonical, the
 * JSON-LD `@id`s, and the sitemap cannot disagree — they previously did, with
 * `https://quinnjr.github.io/armature/` literal in the schemas while the
 * canonical was derived from `Astro.site` plus the configured base.
 */
import { url } from './url';

/** Origin the production site is served from. Mirrors `site` in astro.config.mjs. */
export const SITE_ORIGIN = 'https://quinnjr.github.io';

export const SITE_NAME = 'Armature Framework';
export const AUTHOR_NAME = 'Joseph R. Quinn';
export const AUTHOR_URL = 'https://github.com/quinnjr';
export const REPO_URL = 'https://github.com/quinnjr/armature';

/**
 * Absolute URL for a site-absolute path (base prefix applied).
 *
 * Takes an explicit origin so pages can pass `Astro.site` and stay correct
 * under `astro preview` and any future custom domain.
 */
export function absolute(path: string, origin: string | URL = SITE_ORIGIN): string {
  return new URL(url(path), origin).href;
}

/** One crumb in a breadcrumb trail. `path` is site-absolute, without the base. */
export interface Crumb {
  name: string;
  path: string;
}

/**
 * `BreadcrumbList` JSON-LD for a real trail.
 *
 * The previous site-wide breadcrumb listed Home → Documentation → Getting
 * Started on every page, which is a navigation menu rather than the ancestry of
 * the current page — Google's guidance is that the trail must describe the page
 * it appears on.
 */
export function breadcrumbSchema(crumbs: Crumb[], origin: string | URL = SITE_ORIGIN): string {
  return JSON.stringify({
    '@context': 'https://schema.org',
    '@type': 'BreadcrumbList',
    itemListElement: crumbs.map((crumb, i) => ({
      '@type': 'ListItem',
      position: i + 1,
      name: crumb.name,
      item: absolute(crumb.path, origin),
    })),
  });
}

export interface ArticleSchemaInput {
  title: string;
  description: string;
  path: string;
  /** ISO-8601. Sourced from the markdown file's last git commit. */
  dateModified?: string;
  /** Section/category the article belongs to, e.g. 'Routing'. */
  section?: string;
}

/**
 * `TechArticle` JSON-LD for a documentation page.
 *
 * `TechArticle` rather than `Article`: it is the type schema.org defines for
 * technical documentation, and it is what answer engines look for when deciding
 * whether a page is reference material worth citing.
 */
export function techArticleSchema(input: ArticleSchemaInput, origin: string | URL = SITE_ORIGIN): string {
  const canonical = absolute(input.path, origin);
  return JSON.stringify({
    '@context': 'https://schema.org',
    '@type': 'TechArticle',
    '@id': canonical,
    headline: input.title,
    name: input.title,
    description: input.description,
    url: canonical,
    inLanguage: 'en',
    ...(input.dateModified ? { dateModified: input.dateModified } : {}),
    ...(input.section ? { articleSection: input.section } : {}),
    author: { '@type': 'Person', name: AUTHOR_NAME, url: AUTHOR_URL },
    publisher: { '@type': 'Organization', name: SITE_NAME, url: absolute('/', origin) },
    isPartOf: {
      '@type': 'WebSite',
      name: SITE_NAME,
      url: absolute('/', origin),
    },
    about: {
      '@type': 'SoftwareApplication',
      name: SITE_NAME,
      applicationCategory: 'DeveloperApplication',
    },
    proficiencyLevel: 'Beginner',
    mainEntityOfPage: { '@type': 'WebPage', '@id': canonical },
  });
}

/** `FAQPage` JSON-LD built from the same entries the page renders. */
export function faqSchema(entries: { question: string; answer: string }[]): string {
  return JSON.stringify({
    '@context': 'https://schema.org',
    '@type': 'FAQPage',
    mainEntity: entries.map((entry) => ({
      '@type': 'Question',
      name: entry.question,
      acceptedAnswer: { '@type': 'Answer', text: entry.answer },
    })),
  });
}

export interface CollectionSchemaInput {
  name: string;
  description: string;
  path: string;
  items: { name: string; path: string }[];
}

/** `CollectionPage` + `ItemList` for an index page, so crawlers see the set. */
export function collectionSchema(input: CollectionSchemaInput, origin: string | URL = SITE_ORIGIN): string {
  const canonical = absolute(input.path, origin);
  return JSON.stringify({
    '@context': 'https://schema.org',
    '@type': 'CollectionPage',
    '@id': canonical,
    name: input.name,
    description: input.description,
    url: canonical,
    inLanguage: 'en',
    isPartOf: { '@type': 'WebSite', name: SITE_NAME, url: absolute('/', origin) },
    mainEntity: {
      '@type': 'ItemList',
      numberOfItems: input.items.length,
      itemListElement: input.items.map((item, i) => ({
        '@type': 'ListItem',
        position: i + 1,
        name: item.name,
        url: absolute(item.path, origin),
      })),
    },
  });
}
