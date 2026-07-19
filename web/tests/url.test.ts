import { describe, expect, it } from 'vitest';
import { joinBase, url } from '../src/lib/url';

describe('joinBase', () => {
  it('handles the dev base', () => {
    expect(joinBase('/', '/docs')).toBe('/docs');
    expect(joinBase('/', 'docs')).toBe('/docs');
    expect(joinBase('/', '/')).toBe('/');
  });

  it('handles the GitHub Pages base', () => {
    expect(joinBase('/armature', '/docs')).toBe('/armature/docs');
    expect(joinBase('/armature/', '/docs')).toBe('/armature/docs');
    expect(joinBase('/armature', '/')).toBe('/armature/');
    expect(joinBase('/armature', 'docs/auth-guide')).toBe('/armature/docs/auth-guide');
  });
});

// vitest does not set BASE_URL the way Astro does; vite defaults it to '/'
describe('url helper', () => {
  it('prefixes paths with the configured base', () => {
    expect(url('/docs')).toBe('/docs');
    expect(url('docs')).toBe('/docs');
    expect(url('/')).toBe('/');
  });
});
