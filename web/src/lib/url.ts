/**
 * Join a base path and an absolute site path. Pure so both the dev base (`/`)
 * and the GitHub Pages base (`/armature`) can be unit-tested.
 */
export function joinBase(base: string, path: string): string {
  const b = base.replace(/\/+$/, '');
  return `${b}${path.startsWith('/') ? path : `/${path}`}`;
}

/**
 * Prefix an absolute site path with the configured base path
 * (`/` in dev, `/armature` on GitHub Pages).
 */
export function url(path: string): string {
  return joinBase(import.meta.env.BASE_URL, path);
}
