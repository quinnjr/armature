/**
 * Prefix an absolute site path with the configured base path
 * (`/` in dev, `/armature` on GitHub Pages).
 */
export function url(path: string): string {
  const base = import.meta.env.BASE_URL.replace(/\/+$/, '');
  return `${base}${path.startsWith('/') ? path : `/${path}`}` || '/';
}
