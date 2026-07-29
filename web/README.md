# Armature Framework - Web Documentation

This is the official documentation website for the Armature Framework, built with Astro 5 and Tailwind CSS 4+.

## About Armature

**Armature** is a modern, type-safe HTTP framework for Rust that brings the best architectural patterns from **NestJS**, **Express**, and **Koa** to the Rust ecosystem. Perfect for:

- 🔄 **Node.js developers** migrating to Rust (NestJS, Express, Koa, Fastify users)
- 🦀 **Rust developers** seeking higher-level abstractions (Actix-web, Rocket, Axum alternatives)
- 🏢 **Enterprise teams** needing built-in auth, validation, caching, and observability

### Framework Comparisons

| Feature | Armature | NestJS | Express | Actix-web | Rocket |
|---------|----------|--------|---------|-----------|--------|
| Language | Rust | TypeScript | JavaScript | Rust | Rust |
| DI Container | ✅ | ✅ | ❌ | ❌ | ❌ |
| Decorators | ✅ | ✅ | ❌ | ❌ | ✅ |
| Type Safety | Compile-time | Runtime | None | Compile-time | Compile-time |
| Built-in Auth | ✅ | ✅ | ❌ | ❌ | ❌ |
| OpenAPI | ✅ | ✅ | ❌ | ❌ (utoipa) | ❌ (okapi) |
| Rate Limiting | ✅ | ✅ | ❌ | ❌ | ❌ |

### Key Features

- 🎯 **Familiar Patterns**: Decorators, dependency injection, modules (like NestJS)
- 🚀 **High Performance**: Native Rust performance and memory safety
- 🛡️ **Type Safety**: Compile-time guarantees, not runtime checks
- 🔐 **Built-in Auth**: JWT, OAuth2, SAML support out of the box
- 📚 **OpenAPI/Swagger**: Automatic API documentation generation
- ⚡ **Rate Limiting**: Multiple algorithms with Redis support
- 💼 **Enterprise Ready**: Caching, queues, validation, observability

## Technology Stack

- **Astro**: 5.x (static site generation)
- **Tailwind CSS**: 4.1+ (CSS-first configuration) via `@tailwindcss/vite`
- **Theme**: [`tailswatch`](https://github.com/quinnjr/tailswatch) Oxide theme
- **SCSS**: For enhanced styling capabilities
- **TypeScript**: 5.9+
- **Package Manager**: pnpm
- **Tests**: Vitest

## Tailwind CSS 4+ Configuration

This project uses Tailwind CSS 4's CSS-first configuration approach. No `tailwind.config.js`.

### How it works:

1. **Direct CSS Import**: Tailwind and the Tailswatch Oxide theme are imported in `src/styles/global.css`:
   ```css
   @import 'tailwindcss';
   @import 'tailswatch/themes/oxide';
   ```
   (Keep this file plain CSS — a sass entry point would compile the imports away before Tailwind's Vite plugin runs, silently dropping every utility class.)

2. **Theme Configuration**: Custom theme extensions are defined using the `@theme` directive with CSS variables (see the rust/stone color aliases in `global.css`).

3. **Utility Classes**: Use Tailwind utilities directly in `.astro` templates.

## Documentation Content

The guide pages under `/docs/<id>` are statically generated at build time from the
markdown files in the repository's top-level `docs/` directory. The registry mapping
doc ids to markdown files, titles, and sidebar categories lives in `src/lib/docs.ts`.
Adding a new guide means adding the markdown file to `docs/` and one entry to that registry.

## Development

```bash
# Install dependencies
pnpm install

# Start development server
pnpm start

# Build for production
pnpm run build

# Run tests
pnpm test

# Run linting
pnpm lint
```

## Project Structure

```
web/
├── src/
│   ├── layouts/
│   │   └── BaseLayout.astro    # HTML shell, SEO meta, nav + footer
│   ├── components/
│   │   ├── Nav.astro
│   │   ├── Footer.astro
│   │   ├── Icon.astro          # FontAwesome SVG renderer
│   │   └── docs/               # Docs shell + hand-built doc pages
│   ├── pages/                  # File-based routes
│   │   ├── index.astro
│   │   ├── getting-started.astro
│   │   ├── comparisons.astro
│   │   ├── roadmap.astro
│   │   ├── faq.astro
│   │   ├── 404.astro
│   │   └── docs/
│   │       ├── index.astro
│   │       └── [id].astro      # Statically generated from ../docs/*.md
│   ├── lib/
│   │   ├── docs.ts             # Doc registry (ids, titles, categories, files)
│   │   ├── overview.ts         # Docs overview page data
│   │   ├── markdown.ts         # Build-time markdown rendering + link rewriting
│   │   ├── nav.ts              # Nav links + active-path logic
│   │   ├── schema.ts           # JSON-LD structured data constants
│   │   └── url.ts              # Base-path-aware link helper
│   └── styles/
│       ├── global.css          # Tailwind + Tailswatch Oxide theme + overrides
│       └── docs.scss           # Docs shell + markdown typography
├── tests/                      # Vitest tests
├── public/                     # Static assets (favicon, logos, manifest, ...)
└── astro.config.mjs            # Astro configuration
```

## Customizing Styles

### Adding Custom Colors

Edit `src/styles/global.css`:

```css
@theme {
  --color-accent-500: #f59e0b;
  --color-accent-600: #d97706;
}
```

Then use in templates:

```html
<div class="bg-accent-500 text-white">Custom color</div>
```

## Deployment

### Build for Production

```bash
pnpm run build
```

Output will be in `dist/`.

### Deploy to GitHub Pages

The site is configured for automatic deployment via GitHub Actions (`.github/workflows/docs.yml`).
The `build:gh-pages` script sets the `/armature` base path.

Live site: https://quinnjr.github.io/armature/

## Browser Support

- Chrome/Edge (last 2 versions)
- Firefox (last 2 versions)
- Safari (last 2 versions)
- Mobile browsers (iOS Safari, Chrome Mobile)

## License

Apache 2.0 License - See root LICENSE file for details.

## Contributing

See root CONTRIBUTING.md for contribution guidelines.
