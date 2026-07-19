// @ts-check
import { defineConfig } from 'astro/config';
import tailwindcss from '@tailwindcss/vite';

// ASTRO_BASE is set to /armature for GitHub Pages builds (see build:gh-pages script)
export default defineConfig({
  site: 'https://pegasusheavy.github.io',
  base: process.env.ASTRO_BASE ?? '/',
  vite: {
    plugins: [tailwindcss()],
  },
});
