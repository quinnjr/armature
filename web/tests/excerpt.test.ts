import { describe, expect, it } from 'vitest';
import { excerptFromMarkdown, truncate } from '../src/lib/excerpt';

describe('excerptFromMarkdown', () => {
  it('takes the first prose paragraph, not the H1', () => {
    const md = '# Kubernetes Guide\n\nThis guide covers deploying and operating Armature applications on Kubernetes.\n';
    expect(excerptFromMarkdown(md)).toBe(
      'This guide covers deploying and operating Armature applications on Kubernetes.',
    );
  });

  /**
   * Regression: the block-marker patterns used `\s{0,3}` for the leading
   * indent, and `\s` matches a newline — so stripping `## Table of Contents`
   * also consumed the blank line above it, splicing the heading and its list
   * onto the end of the intro paragraph.
   */
  it('does not splice a following heading into the paragraph', () => {
    const md = [
      '# Kubernetes Guide',
      '',
      'This guide covers deploying and operating Armature applications on Kubernetes.',
      '',
      '## Table of Contents',
      '',
      '- [Overview](#overview)',
      '- [Basic Deployment](#basic-deployment)',
      '',
    ].join('\n');
    expect(excerptFromMarkdown(md)).toBe(
      'This guide covers deploying and operating Armature applications on Kubernetes.',
    );
  });

  it('never quotes fenced code', () => {
    const md = '# Title\n\n```rust\nlet secret = "do not put me in a search result";\n```\n\nThe prose paragraph that should win instead.\n';
    expect(excerptFromMarkdown(md)).toBe('The prose paragraph that should win instead.');
  });

  it('skips badge rows and tables', () => {
    const md = [
      '# Title',
      '',
      '[![build](https://img.shields.io/x)](https://example.com) [![docs](https://img.shields.io/y)](https://example.com)',
      '',
      '| Column | Other |',
      '|---|---|',
      '| a | b |',
      '',
      'Armature provides a robust job queue for background processing of asynchronous tasks.',
      '',
    ].join('\n');
    expect(excerptFromMarkdown(md)).toBe(
      'Armature provides a robust job queue for background processing of asynchronous tasks.',
    );
  });

  it('unwraps links and inline formatting to their visible text', () => {
    const md = 'See the **[configuration guide](config-guide.md)** for the `full` feature flag and how it changes builds.\n';
    expect(excerptFromMarkdown(md)).toBe(
      'See the configuration guide for the full feature flag and how it changes builds.',
    );
  });

  it('returns null when there is no prose worth quoting', () => {
    expect(excerptFromMarkdown('# Title\n\n```\ncode only\n```\n')).toBeNull();
    expect(excerptFromMarkdown('')).toBeNull();
  });
});

describe('truncate', () => {
  it('leaves text within budget untouched', () => {
    expect(truncate('Short enough.', 155)).toBe('Short enough.');
  });

  it('prefers a sentence boundary when one lands late enough', () => {
    const text = 'A'.repeat(70) + '. And then a second sentence that runs past the budget entirely.';
    const out = truncate(text, 100);
    expect(out.endsWith('.')).toBe(true);
    expect(out.length).toBeLessThanOrEqual(101);
  });

  it('falls back to a word boundary with an ellipsis', () => {
    const out = truncate('alpha bravo charlie delta echo foxtrot golf hotel india', 20);
    expect(out.endsWith('…')).toBe(true);
    expect(out.length).toBeLessThanOrEqual(20);
    expect(out).not.toContain('  ');
  });
});
