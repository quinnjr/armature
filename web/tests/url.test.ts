import { describe, expect, it } from 'vitest';
import { url } from '../src/lib/url';

// vitest does not set BASE_URL the way Astro does; vite defaults it to '/'
describe('url helper', () => {
  it('prefixes paths with the base', () => {
    expect(url('/docs')).toBe('/docs');
    expect(url('docs')).toBe('/docs');
    expect(url('/')).toBe('/');
  });
});
