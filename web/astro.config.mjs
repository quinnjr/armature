// @ts-check
import { defineConfig } from 'astro/config';
import sitemap from '@astrojs/sitemap';
import tailwindcss from '@tailwindcss/vite';

// ASTRO_BASE is set to /armature for GitHub Pages builds (see build:gh-pages script).
// File output + no trailing slash preserves the pre-Astro URL contract
// (/armature/docs/auth-guide, exact, no redirect) on GitHub Pages.
export default defineConfig({
  site: 'https://pegasusheavy.github.io',
  base: process.env.ASTRO_BASE ?? '/',
  trailingSlash: 'never',
  build: {
    format: 'file',
  },
  integrations: [sitemap()],
  vite: {
    plugins: [tailwindcss()],
  },
});
