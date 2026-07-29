export interface NavLink {
  /** Site-absolute path without the base prefix. */
  href: string;
  label: string;
  mobileLabel: string;
  /** Match only the exact path instead of the path and its descendants. */
  exact: boolean;
}

export const navLinks: NavLink[] = [
  { href: '/', label: 'Home', mobileLabel: 'Home', exact: true },
  { href: '/docs', label: 'Docs', mobileLabel: 'Documentation', exact: false },
  { href: '/comparisons', label: 'Comparisons', mobileLabel: 'Comparisons', exact: false },
  { href: '/roadmap', label: 'Roadmap', mobileLabel: 'Roadmap', exact: false },
  { href: '/getting-started', label: 'Getting Started', mobileLabel: 'Getting Started', exact: false },
  { href: '/faq', label: 'FAQ', mobileLabel: 'FAQ', exact: false },
];

/**
 * Whether a nav target (already base-prefixed) is active for the current
 * pathname. Trailing slashes are ignored; non-exact targets also match
 * descendant paths but not sibling prefixes (`/docs` ≠ `/docs-archive`).
 */
export function isActivePath(currentPath: string, target: string, exact: boolean): boolean {
  const t = target.replace(/\/+$/, '') || '/';
  const p = currentPath.replace(/\/+$/, '') || '/';
  return exact ? p === t : p === t || p.startsWith(`${t}/`);
}
