import { describe, expect, it } from 'vitest';
import { isActivePath, navLinks } from '../src/lib/nav';

describe('isActivePath', () => {
  it.each([
    // [currentPath, target, exact, expected]
    ['/', '/', true, true],
    ['/docs', '/', true, false],
    ['/docs', '/docs', false, true],
    ['/docs/auth-guide', '/docs', false, true],
    ['/docs/', '/docs', false, true],
    ['/docs-archive', '/docs', false, false],
    ['/armature/docs/auth-guide', '/armature/docs', false, true],
    ['/armature', '/armature', true, true],
    ['/armature/docs', '/armature', true, false],
  ])('isActivePath(%j, %j, exact=%j) -> %j', (currentPath, target, exact, expected) => {
    expect(isActivePath(currentPath, target, exact)).toBe(expected);
  });
});

describe('navLinks', () => {
  it('marks only Home as exact', () => {
    expect(navLinks.filter((l) => l.exact).map((l) => l.href)).toEqual(['/']);
  });
});
