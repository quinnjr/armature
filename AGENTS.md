# AGENTS.md

Instructions for AI coding agents working on the Armature framework.

## Project Overview

Armature is a type-safe HTTP framework for Rust inspired by Angular and NestJS. It combines decorator syntax (via proc macros) and dependency injection with Rust's performance and safety. The codebase is a Cargo workspace with 60+ crates.

## Build & Test Commands

```bash
# Build (without SAML — use this by default)
cargo build --features full

# Build (with SAML — requires libxml2-dev, libxmlsec1-dev, libxmlsec1-openssl)
cargo build --features full-with-saml

# Run all tests
cargo test --features full

# Run doc tests only
cargo test --doc --features full

# Run a specific crate's tests
cargo test -p armature-core --features full

# Format check
cargo fmt -- --check

# Lint (allowed warnings match CI config)
cargo clippy --all-targets --features full -- -D warnings \
  -A clippy::collapsible_if \
  -A clippy::result_large_err \
  -A dead_code \
  -A clippy::useless_vec \
  -A clippy::unwrap_or_default

# Lint the per-crate benchmarks. The command above is scoped to the root
# package, which no longer owns them, so it does NOT cover these.
#
# ONE CRATE AT A TIME: cargo unifies a dependency's features across every
# package in a single `-p` selection, so a combined run lets one crate's
# criterion features satisfy a sibling whose manifest omits them. These are
# separately published crates and each must build from its own manifest alone.
for c in $(cargo metadata --no-deps --format-version 1 \
    | jq -r '.packages[] | select([.targets[].kind[]] | index("bench")) | .name' \
    | grep -v '^armature-framework$'); do
  cargo clippy --benches -p "$c" -- -D warnings
done

# Run benchmarks (each one is owned by the crate it measures)
./scripts/run-benchmarks.sh --all
cargo bench -p armature-core --bench internal_overhead
```

**SAML is optional.** Most development uses `--features full` without SAML. Only use `full-with-saml` when working on SAML-related code and you have the system libraries installed.

## Repository Structure

```
armature-framework/          # Workspace root, Cargo.toml defines all members
├── armature-core/           # HTTP routing, middleware, DI container, Application bootstrap
├── armature-proc-macro/     # Procedural macros: #[controller], #[get], #[injectable], #[module]
├── armature-log/            # Structured logging
├── armature-auth/           # JWT, OAuth2, SAML, RBAC, guards
├── armature-jwt/            # JWT token management (HS256/RS256/ES256)
├── armature-security/       # CORS, CSP, HSTS
├── armature-config/         # Type-safe config (env, .env, JSON, TOML)
├── armature-cache/          # Redis/Memcached/in-memory caching
├── armature-redis/          # Centralized Redis client
├── armature-queue/          # Background job queues
├── armature-events/         # Event bus (pub/sub)
├── armature-eventsourcing/  # Event sourcing, projections, snapshots
├── armature-cqrs/           # Command/Query Responsibility Segregation
├── armature-graphql/        # GraphQL server (schema-first and code-first)
├── armature-openapi/        # OpenAPI/Swagger generation
├── armature-opentelemetry/  # Distributed tracing (OTLP, Zipkin), metrics
├── armature-websocket/      # WebSocket with rooms and broadcasting
├── armature-messaging/      # RabbitMQ, Kafka, NATS
├── armature-aws/            # AWS SDK (S3, DynamoDB, SQS, SNS, Lambda, etc.)
├── armature-gcp/            # GCP SDK (Storage, Pub/Sub, Firestore, BigQuery)
├── armature-azure/          # Azure SDK (Blob, Cosmos, Service Bus, Key Vault)
├── armature-lambda/         # AWS Lambda integration
├── armature-cloudrun/       # GCP Cloud Run integration
├── armature-azure-functions/# Azure Functions integration
├── armature-cli/            # Code generation & dev server CLI
├── armature-testing/        # Testing utilities, mocks, spies
├── armature-validation/     # Validation framework
├── armature-ratelimit/      # Rate limiting (token bucket, sliding window)
├── armature-compression/    # gzip/brotli/zstd compression
├── armature-distributed/    # Distributed locks, leader election
├── armature-discovery/      # Service discovery (Consul, etcd)
├── armature-toon/           # Token-optimized serialization for LLMs
├── armature-ferron/         # Custom Rhai scripting engine
├── armature-rhai/           # Embedded Rhai scripting
├── armature-diesel/         # Diesel ORM integration
├── armature-seaorm/         # SeaORM integration
├── armature-storage/        # Cloud file/blob storage
├── armature-mail/           # Email sending
├── armature-push/           # Push notifications
├── armature-payments/       # Payment processing (Stripe, PayPal)
├── armature-admin/          # Auto-generated admin dashboard
├── armature-collab/         # Real-time collaboration (CRDTs)
├── armature-analytics/      # Analytics pipeline
├── armature-siem/           # Security info & event management
├── armature-files/          # File upload/processing
├── armature-tenancy/        # Multi-tenancy
├── armature-features/       # Feature flags
├── armature-opensearch/     # Full-text search
├── armature-i18n/           # Internationalization
├── armature-metrics/        # Prometheus metrics
├── armature-audit/          # Audit logging
├── armature-webhooks/       # Webhook handling
├── armature-cron/           # Scheduled tasks
├── armature-acme/           # Let's Encrypt certificates
├── armature-http-client/    # HTTP client
├── armature-grpc/           # gRPC integration
├── armature-graphql-client/ # GraphQL client
├── armature-app/            # Build full Armature apps in Rhai scripts (zero Rust)
├── armature-macros/         # Additional macros
├── armature-macros-utils/   # Macro utilities
├── docs/                    # 70+ guides
├── examples/                # 60+ working examples
├── benches/                 # Cross-framework comparison harness (comparison_servers/, techempower/,
│                           # http-benchmark) + the database/memory pattern benches. Per-crate
│                           # criterion benches live in each crate's own benches/.
├── tests/                   # Integration tests
└── templates/               # Project scaffolding templates (excluded from workspace)
```

## Architecture Patterns

The framework follows NestJS/Angular conventions adapted to Rust:

- **Decorators** are proc macros: `#[controller]`, `#[get]`, `#[post]`, `#[put]`, `#[delete]`, `#[patch]`, `#[options]`, `#[head]`, `#[query]`, `#[injectable]`, `#[module]`
- **HTTP methods**: `HttpMethod` includes `QUERY` (IETF safe-method-with-body). It is `#[non_exhaustive]` — always include a `_` arm when matching it.
- **Dependency injection** is field-based — add a service type as a struct field and it's auto-injected
- **Modules** group providers (services) and controllers with `#[module(...)]`
- **Application bootstrap** via `Application::create::<AppModule>().await`
- **Guards** implement the `Guard` trait for authorization. They **fail closed**: a `RoleGuard`/`PermissionGuard` requires a verified `UserContext`/`RequestRoles` extension attached by an authentication layer (use `armature_auth::JwtAuthMiddleware`, which verifies the JWT and populates them). A module's guards are scoped to that module's controllers' routes, not applied globally.
- **Middleware** implements the `Middleware` trait for request/response pipeline
- **Lifecycle hooks**: `OnModuleInit`, `OnModuleDestroy`, `OnApplicationBootstrap`, `OnApplicationShutdown`
- **Routing**: the linear `Router` is the registration target; it is compiled once into an O(1) `OptimizedRouter` for the serve path. Preserve exact routing semantics (param extraction, catch-all, constraints, unknown-method → 404) when touching either.

## Key Conventions

- **Rust 2024 edition**, MSRV 1.94.1
- **Async-first**: Built on Tokio + Hyper. All handlers are `async`
- **Feature flags**: The crate uses feature flags extensively. `full` enables everything except SAML. `full-with-saml` enables everything
- **`tokio` features**: crates declare the minimal per-crate feature subset they use (e.g. `["rt", "macros", "sync", "time"]`, plus `net`/`io-util`/`fs` as needed) — do **not** use `features = ["full"]`
- **TLS is rustls-only**: do not pull `native-tls`/OpenSSL. Add `default-features = false` to `reqwest`/`tokio-tungstenite` deps and select the rustls feature
- `HttpRequest.headers` is a `HeaderMap` (SmallVec-backed, case-insensitive); it is a drop-in for the old `HashMap<String,String>` access patterns
- Core types: `HttpRequest`, `HttpResponse`, `Router`, `Container`, `Application`, `Error`
- Error type has 30+ variants with status codes, help text, and client/server classification
- Response builder is fluent: `HttpResponse::ok().json(&data)?`
- Extractors use attribute macros: `#[body]`, `#[param("id")]`, `#[query("page")]`, `#[header("authorization")]`
- Services are singletons — created once, shared via `Arc`
- **Changelogs are per-crate**: each crate keeps its own `CHANGELOG.md` (e.g. `armature-core/CHANGELOG.md`) in [Keep a Changelog](https://keepachangelog.com/en/1.0.0/) format with an `## [Unreleased]` section at the top. Write new entries there — the root `CHANGELOG.md` is the historical record through `0.3.0` plus workspace-wide notes, not a place for new per-crate entries. A crate with no `include`/`exclude` in its `Cargo.toml` packages its `CHANGELOG.md` automatically; no manifest change needed

## Git Workflow

- **`main`** — stable release branch, target for PRs
- **`develop`** — active development branch
- Branch naming: `feature/*`, `bugfix/*`
- CI runs on push to `main`/`develop` and on all PRs
- CI checks: format, clippy, tests (Linux/macOS/Windows, stable/beta/nightly), doc tests, example builds

## Performance Notes

- Target: Actix-competitive performance (currently 242k req/sec plaintext)
- JSON serialization is a known optimization area
- Criterion benchmarks live in the crate they measure (`armature-core/benches/`, `armature-jwt/benches/`, ...), so they are always run `-p`-scoped: `cargo bench -p armature-core --bench internal_overhead`. `scripts/run-benchmarks.sh` runs them by suite. Those crates set `autobenches = false`, so a new bench file needs an explicit `[[bench]]` entry
- The root `benches/` keeps only what is not crate-specific: cross-framework HTTP comparison (`benches/comparison_servers/` + the `http-benchmark` runner, `benches/techempower/`) and the `database_benchmarks`/`memory_benchmarks` pattern benchmarks. Profiling (flamegraphs, DHAT/pprof) is not in `benches/` — it lives in `examples/profiling_server.rs`, `examples/memory_profile_server.rs`, and `scripts/memory-profile.sh`
- Do not regress performance without justification — run the relevant benchmarks before and after changes

## When Making Changes

1. Run `cargo fmt` before committing
2. Run clippy with the CI flags shown above — do not introduce new warnings
3. Run `cargo test --features full` to validate
4. If adding a new crate, add it to the workspace `members` in root `Cargo.toml`
5. If adding public API, add doc comments and a doc test; if bumping a crate's version, record the change in that crate's own `CHANGELOG.md`
6. If adding a new feature, add an example in `examples/` and a guide in `docs/`
7. Keep the NestJS/Angular decorator-style patterns consistent — don't introduce foreign paradigms

---

<!-- converted from Cursor rules -->

## Cursor rule: `.cursor/rules/angular-docs-website.mdc`

# Angular Documentation Website

Guidelines for developing the Armature documentation website in the `web/` directory.

## Technology Stack

- **Framework:** Angular 21 (standalone components)
- **Package Manager:** pnpm (required)
- **Testing:** Vitest
- **Styling:** CSS/SCSS

## Project Structure

```
web/
├── src/
│   ├── app/
│   │   ├── components/     # Shared UI components
│   │   ├── pages/          # Route pages
│   │   ├── services/       # Angular services
│   │   ├── models/         # TypeScript interfaces
│   │   └── app.component.ts
│   ├── assets/             # Static assets
│   └── styles/             # Global styles
├── public/                 # Public static files
├── angular.json
├── package.json
├── vitest.config.ts
└── tsconfig.json
```

## Angular 21 Patterns

### Standalone Components (Required)

All components must be standalone:

```typescript
// ✅ Good: Standalone component
@Component({
  selector: 'app-feature',
  standalone: true,
  imports: [CommonModule, RouterModule],
  template: `...`,
})
export class FeatureComponent { }

// ❌ Bad: Non-standalone (deprecated pattern)
@Component({
  selector: 'app-feature',
  template: `...`,
})
export class FeatureComponent { }
// Then added to NgModule declarations
```

### Signals (Preferred for State)

```typescript
import { signal, computed, effect } from '@angular/core';

@Component({
  selector: 'app-counter',
  standalone: true,
  template: `
    <button (click)="increment()">Count: {{ count() }}</button>
    <p>Double: {{ doubleCount() }}</p>
  `,
})
export class CounterComponent {
  count = signal(0);
  doubleCount = computed(() => this.count() * 2);

  constructor() {
    effect(() => {
      console.log('Count changed:', this.count());
    });
  }

  increment() {
    this.count.update(c => c + 1);
  }
}
```

### New Control Flow Syntax

```typescript
// ✅ Good: New control flow (Angular 17+)
@Component({
  template: `
    @if (isLoading()) {
      <app-spinner />
    } @else if (error()) {
      <app-error [message]="error()" />
    } @else {
      @for (item of items(); track item.id) {
        <app-item [data]="item" />
      } @empty {
        <p>No items found</p>
      }
    }

    @switch (status()) {
      @case ('pending') { <span>Pending...</span> }
      @case ('success') { <span>Success!</span> }
      @default { <span>Unknown</span> }
    }
  `,
})
export class ListComponent {
  items = signal<Item[]>([]);
  isLoading = signal(false);
  error = signal<string | null>(null);
  status = signal<'pending' | 'success' | 'error'>('pending');
}

// ❌ Bad: Old structural directives
@Component({
  template: `
    <ng-container *ngIf="isLoading; else loaded">
      <app-spinner></app-spinner>
    </ng-container>
    <ng-template #loaded>
      <app-item *ngFor="let item of items; trackBy: trackById" [data]="item"></app-item>
    </ng-template>
  `,
})
```

### Inject Function (Preferred)

```typescript
// ✅ Good: inject() function
@Component({
  selector: 'app-feature',
  standalone: true,
  template: `...`,
})
export class FeatureComponent {
  private http = inject(HttpClient);
  private route = inject(ActivatedRoute);
  private docsService = inject(DocsService);
}

// ❌ Less preferred: Constructor injection
@Component({
  selector: 'app-feature',
  standalone: true,
  template: `...`,
})
export class FeatureComponent {
  constructor(
    private http: HttpClient,
    private route: ActivatedRoute,
    private docsService: DocsService,
  ) { }
}
```

## Commands

```bash
# Development server
cd web
pnpm start          # Starts on http://localhost:4200

# Build for production
pnpm run build

# Run tests
pnpm test           # Uses Vitest

# Lint
pnpm run lint
```

## Documentation Content Integration

### Loading Markdown Docs

The website should load and render markdown documentation from `docs/`:

```typescript
@Injectable({ providedIn: 'root' })
export class DocsService {
  private http = inject(HttpClient);

  getDoc(slug: string): Observable<string> {
    return this.http.get(`/docs/${slug}.md`, { responseType: 'text' });
  }
}
```

### Code Syntax Highlighting

Use Prism.js or highlight.js for Rust code highlighting:

```typescript
@Component({
  selector: 'app-code-block',
  standalone: true,
  template: `
    <pre><code [innerHTML]="highlightedCode()"></code></pre>
  `,
})
export class CodeBlockComponent {
  code = input.required<string>();
  language = input<string>('rust');

  highlightedCode = computed(() => {
    return Prism.highlight(
      this.code(),
      Prism.languages[this.language()],
      this.language()
    );
  });
}
```

## Styling Guidelines

### CSS Variables for Theming

```css
:root {
  /* Colors */
  --color-primary: #ff6b35;
  --color-secondary: #3498db;
  --color-background: #1a1a2e;
  --color-surface: #16213e;
  --color-text: #eaeaea;
  --color-text-muted: #888888;

  /* Typography */
  --font-heading: 'JetBrains Mono', monospace;
  --font-body: 'Inter', sans-serif;
  --font-code: 'Fira Code', monospace;

  /* Spacing */
  --space-xs: 0.25rem;
  --space-sm: 0.5rem;
  --space-md: 1rem;
  --space-lg: 2rem;
  --space-xl: 4rem;
}
```

### Responsive Design

```css
/* Mobile-first approach */
.container {
  padding: var(--space-md);
}

@media (min-width: 768px) {
  .container {
    padding: var(--space-lg);
  }
}

@media (min-width: 1024px) {
  .container {
    max-width: 1200px;
    margin: 0 auto;
  }
}
```

## Component Library

### Documentation-Specific Components

Create reusable components for documentation:

```typescript
// Callout/Admonition component
@Component({
  selector: 'app-callout',
  standalone: true,
  template: `
    <div [class]="'callout callout-' + type()">
      <div class="callout-icon">{{ icon() }}</div>
      <div class="callout-content">
        <ng-content />
      </div>
    </div>
  `,
})
export class CalloutComponent {
  type = input<'info' | 'warning' | 'danger' | 'tip'>('info');

  icon = computed(() => {
    const icons = { info: 'ℹ️', warning: '⚠️', danger: '🚨', tip: '💡' };
    return icons[this.type()];
  });
}
```

## Testing with Vitest

```typescript
// feature.component.spec.ts
import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/angular';
import { FeatureComponent } from './feature.component';

describe('FeatureComponent', () => {
  it('should render title', async () => {
    await render(FeatureComponent, {
      inputs: { title: 'Test Title' },
    });

    expect(screen.getByText('Test Title')).toBeTruthy();
  });

  it('should handle click events', async () => {
    const { fixture } = await render(FeatureComponent);
    const button = screen.getByRole('button');

    button.click();
    fixture.detectChanges();

    expect(screen.getByText('Clicked!')).toBeTruthy();
  });
});
```

## Deployment

The website automatically deploys to GitHub Pages when changes are merged to `main`.

### Manual Build

```bash
cd web
pnpm run build
# Output in web/dist/web/browser/
```

## Accessibility

- All interactive elements must have proper ARIA labels
- Use semantic HTML elements
- Ensure sufficient color contrast
- Support keyboard navigation
- Include skip links for navigation

## Performance

- Lazy load route components
- Optimize images with WebP format
- Use `@defer` for heavy components
- Implement virtual scrolling for long lists

```typescript
// Lazy loading with @defer
@Component({
  template: `
    @defer (on viewport) {
      <app-heavy-component />
    } @placeholder {
      <div class="skeleton"></div>
    } @loading (minimum 500ms) {
      <app-spinner />
    }
  `,
})
export class PageComponent { }
```

## Summary

1. Use **standalone components** exclusively
2. Prefer **signals** over BehaviorSubject/observables for state
3. Use **new control flow** syntax (@if, @for, @switch)
4. Use **inject()** function for dependency injection
5. Use **pnpm** as package manager
6. Test with **Vitest**
7. Follow **mobile-first** responsive design
8. Ensure **accessibility** compliance


## Cursor rule: `.cursor/rules/api-design.mdc`

_REST API design guidelines for Armature applications_

Applies to: `**/controllers/**/*.rs, **/routes/**/*.rs, **/handlers/**/*.rs`

# API Design

Guidelines for designing RESTful APIs with Armature.

## URL Structure

```
GET    /api/v1/users          # List users
POST   /api/v1/users          # Create user
GET    /api/v1/users/:id      # Get user
PUT    /api/v1/users/:id      # Replace user
PATCH  /api/v1/users/:id      # Update user
DELETE /api/v1/users/:id      # Delete user

GET    /api/v1/users/:id/posts  # Nested resources
```

## Controller Structure

```rust
#[controller("/api/v1/users")]
pub struct UsersController {
    user_service: Arc<UserService>,
}

#[get("")]
async fn list(&self, query: Query<ListParams>) -> Result<Json<Page<UserResponse>>> {
    let users = self.user_service.list(&query).await?;
    Ok(Json(users.into()))
}

#[get("/:id")]
async fn get(&self, id: Path<Uuid>) -> Result<Json<UserResponse>> {
    let user = self.user_service.get(*id).await?;
    Ok(Json(user.into()))
}

#[post("")]
async fn create(&self, body: Json<CreateUserRequest>) -> Result<Created<Json<UserResponse>>> {
    let user = self.user_service.create(body.into_inner()).await?;
    Ok(Created(Json(user.into())))
}
```

## Request DTOs

```rust
#[derive(Deserialize, Validate)]
pub struct CreateUserRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: String,

    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8))]
    pub password: String,
}

#[derive(Deserialize, Validate)]
pub struct UpdateUserRequest {
    #[validate(length(min = 1, max = 100))]
    pub name: Option<String>,

    #[validate(email)]
    pub email: Option<String>,
}
```

## Response DTOs

```rust
#[derive(Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub created_at: DateTime<Utc>,
}

// Never expose internal models directly
impl From<User> for UserResponse {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            name: user.name,
            email: user.email,
            created_at: user.created_at,
        }
    }
}
```

## Pagination

```rust
#[derive(Deserialize)]
pub struct ListParams {
    #[serde(default = "default_page")]
    pub page: u32,

    #[serde(default = "default_per_page")]
    pub per_page: u32,

    pub sort: Option<String>,
    pub order: Option<SortOrder>,
}

#[derive(Serialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub meta: PageMeta,
}

#[derive(Serialize)]
pub struct PageMeta {
    pub page: u32,
    pub per_page: u32,
    pub total: u64,
    pub total_pages: u32,
}
```

## Error Responses

```rust
#[derive(Serialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Vec<FieldError>>,
}

#[derive(Serialize)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

// Map errors to HTTP status codes
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, response) = match self {
            AppError::NotFound(msg) => (
                StatusCode::NOT_FOUND,
                ErrorResponse { code: "NOT_FOUND".into(), message: msg, details: None }
            ),
            AppError::Validation(errors) => (
                StatusCode::BAD_REQUEST,
                ErrorResponse {
                    code: "VALIDATION_ERROR".into(),
                    message: "Invalid input".into(),
                    details: Some(errors)
                }
            ),
            // ...
        };
        (status, Json(response)).into_response()
    }
}
```

## HTTP Status Codes

| Code | Usage |
|------|-------|
| 200 | Successful GET, PUT, PATCH |
| 201 | Successful POST (resource created) |
| 204 | Successful DELETE (no content) |
| 400 | Validation error |
| 401 | Authentication required |
| 403 | Forbidden (authenticated but not authorized) |
| 404 | Resource not found |
| 409 | Conflict (duplicate resource) |
| 422 | Unprocessable entity |
| 500 | Internal server error |

## OpenAPI Documentation

```rust
#[utoipa::path(
    get,
    path = "/api/v1/users/{id}",
    params(
        ("id" = Uuid, Path, description = "User ID")
    ),
    responses(
        (status = 200, description = "User found", body = UserResponse),
        (status = 404, description = "User not found", body = ErrorResponse)
    ),
    tag = "users"
)]
async fn get_user(id: Path<Uuid>) -> Result<Json<UserResponse>> {
    // ...
}
```


## Cursor rule: `.cursor/rules/auto-commit.mdc`

_Commit message format and auto-commit policy_

# Commit Message Convention

**ALWAYS** use the `type(scope): description` format for ALL commits.

## Format (Required)

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

### Type (Required)

| Type | Description |
|------|-------------|
| `feat` | New feature |
| `fix` | Bug fix |
| `docs` | Documentation changes |
| `refactor` | Code refactoring |
| `test` | Adding or updating tests |
| `chore` | Maintenance tasks, dependencies |
| `style` | Code style/formatting changes |
| `perf` | Performance improvements |
| `ci` | CI/CD configuration |
| `build` | Build system changes |

### Scope (Required)

The scope indicates which part of the codebase is affected:

| Scope | When to use |
|-------|-------------|
| `core` | armature-core crate |
| `auth` | armature-auth, armature-oauth2, armature-saml |
| `cache` | armature-cache, armature-redis |
| `queue` | armature-queue |
| `di` | armature-di |
| `macros` | armature-proc-macro |
| `cli` | armature-cli |
| `ws` | armature-websocket |
| `sse` | armature-sse |
| `db` | armature-diesel, armature-seaorm |
| `aws` | armature-aws |
| `gcp` | armature-gcp |
| `azure` | armature-azure |
| `docs` | Documentation files |
| `examples` | Example code |
| `tests` | Test files |
| `deps` | Dependency updates |

### Examples

```bash
# Features
git commit -m "feat(auth): add OAuth2 provider support"
git commit -m "feat(queue): implement job retry with exponential backoff"
git commit -m "feat(ws): add room-based message broadcasting"

# Bug fixes
git commit -m "fix(cache): resolve Redis connection timeout"
git commit -m "fix(core): handle empty request body correctly"

# Documentation
git commit -m "docs(readme): update installation instructions"
git commit -m "docs(auth): add JWT configuration examples"

# Refactoring
git commit -m "refactor(core): simplify error handling"
git commit -m "refactor(macros): reduce code duplication in derive macros"

# Tests
git commit -m "test(auth): add integration tests for OAuth2 flow"

# Chores
git commit -m "chore(deps): update tokio to 1.35"

# Breaking changes (add ! after scope)
git commit -m "feat(api)!: change response format to camelCase"
```

## Auto-Commit Policy

Commit after completing tasks that modify files:
- ✅ Implementing a feature
- ✅ Fixing a bug
- ✅ Refactoring code
- ✅ Adding/updating documentation
- ✅ Creating new files
- ✅ Modifying configuration

Do NOT commit when:
- ❌ Changes are incomplete or broken
- ❌ Tests are failing
- ❌ User explicitly asks not to commit
- ❌ Only reading files (no modifications)

## Multi-File Commits

When changes span multiple crates, use the most significant scope or `core`:

```bash
# Changes to multiple crates for a single feature
git commit -m "feat(auth): add session management with Redis storage"

# Dependency updates across workspace
git commit -m "chore(deps): update workspace dependencies"
```


## Cursor rule: `.cursor/rules/cli-development.mdc`

_Guidelines for developing the armature-cli tool_

Applies to: `armature-cli/**/*.rs`

# CLI Development

Standards for developing the Armature CLI tool.

## Command Structure

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "armature")]
#[command(about = "Armature Framework CLI")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a new Armature project
    New(NewCommand),

    /// Generate code (controller, service, etc.)
    Generate(GenerateCommand),

    /// Start development server
    Dev(DevCommand),

    /// Build for production
    Build(BuildCommand),
}
```

## New Project Command

```rust
#[derive(Args)]
pub struct NewCommand {
    /// Project name
    pub name: String,

    /// Template to use
    #[arg(short, long, default_value = "default")]
    pub template: String,

    /// Skip git initialization
    #[arg(long)]
    pub no_git: bool,
}

impl NewCommand {
    pub async fn run(&self) -> Result<()> {
        println!("Creating new project: {}", self.name);

        // Create directory structure
        create_project_structure(&self.name)?;

        // Copy template files
        copy_template(&self.template, &self.name)?;

        // Initialize git
        if !self.no_git {
            init_git(&self.name)?;
        }

        println!("✅ Project created successfully!");
        println!("\nNext steps:");
        println!("  cd {}", self.name);
        println!("  cargo run");

        Ok(())
    }
}
```

## Code Generation

```rust
#[derive(Subcommand)]
pub enum GenerateCommand {
    /// Generate a controller
    Controller(GenerateControllerArgs),

    /// Generate a service
    Service(GenerateServiceArgs),

    /// Generate a module
    Module(GenerateModuleArgs),

    /// Generate a migration
    Migration(GenerateMigrationArgs),
}

#[derive(Args)]
pub struct GenerateControllerArgs {
    /// Controller name (e.g., "users" or "api/v1/users")
    pub name: String,

    /// Generate CRUD endpoints
    #[arg(long)]
    pub crud: bool,
}

impl GenerateControllerArgs {
    pub fn run(&self) -> Result<()> {
        let template = if self.crud {
            include_str!("templates/controller_crud.rs.tmpl")
        } else {
            include_str!("templates/controller.rs.tmpl")
        };

        let rendered = render_template(template, &self.context())?;

        let path = format!("src/controllers/{}.rs", self.name.to_snake_case());
        fs::write(&path, rendered)?;

        println!("✅ Created {}", path);

        // Update mod.rs
        update_mod_file("src/controllers/mod.rs", &self.name)?;

        Ok(())
    }
}
```

## Development Server

```rust
#[derive(Args)]
pub struct DevCommand {
    /// Port to listen on
    #[arg(short, long, default_value = "3000")]
    pub port: u16,

    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    pub host: String,

    /// Enable hot reload
    #[arg(long, default_value = "true")]
    pub hot_reload: bool,
}

impl DevCommand {
    pub async fn run(&self) -> Result<()> {
        println!("🚀 Starting development server on {}:{}", self.host, self.port);

        if self.hot_reload {
            // Watch for file changes
            let watcher = FileWatcher::new(vec!["src/**/*.rs"])?;

            loop {
                // Build and run
                let child = Command::new("cargo")
                    .args(["run"])
                    .env("ARMATURE_PORT", self.port.to_string())
                    .spawn()?;

                // Wait for changes
                watcher.wait_for_changes().await?;

                // Restart
                child.kill()?;
                println!("🔄 Restarting...");
            }
        } else {
            Command::new("cargo")
                .args(["run"])
                .status()?;
        }

        Ok(())
    }
}
```

## Template Files

Store templates in `armature-cli/templates/`:

```rust
// templates/controller.rs.tmpl
use armature::prelude::*;

#[controller("/{{path}}")]
pub struct {{name}}Controller {
    // Add dependencies here
}

#[get("")]
async fn list(&self) -> Result<Json<Vec<{{model}}>>, Error> {
    todo!()
}

#[get("/:id")]
async fn get(&self, id: Path<Uuid>) -> Result<Json<{{model}}>, Error> {
    todo!()
}
```

## Output Formatting

```rust
use console::{style, Emoji};

static SUCCESS: Emoji = Emoji("✅", "[OK]");
static ERROR: Emoji = Emoji("❌", "[ERR]");
static INFO: Emoji = Emoji("ℹ️", "[INFO]");

fn print_success(msg: &str) {
    println!("{} {}", SUCCESS, style(msg).green());
}

fn print_error(msg: &str) {
    eprintln!("{} {}", ERROR, style(msg).red());
}

fn print_info(msg: &str) {
    println!("{} {}", INFO, style(msg).cyan());
}
```

## Error Handling

```rust
use miette::{Diagnostic, Result};
use thiserror::Error;

#[derive(Error, Diagnostic, Debug)]
pub enum CliError {
    #[error("Project '{0}' already exists")]
    #[diagnostic(code(armature::project_exists))]
    ProjectExists(String),

    #[error("Template '{0}' not found")]
    #[diagnostic(
        code(armature::template_not_found),
        help("Available templates: default, api, minimal")
    )]
    TemplateNotFound(String),

    #[error("Invalid project name: {0}")]
    #[diagnostic(code(armature::invalid_name))]
    InvalidName(String),
}
```

## Testing CLI Commands

```rust
#[test]
fn test_new_command() {
    let temp = tempdir().unwrap();

    let cmd = NewCommand {
        name: "test-project".into(),
        template: "default".into(),
        no_git: true,
    };

    std::env::set_current_dir(&temp).unwrap();
    cmd.run().unwrap();

    assert!(temp.path().join("test-project/Cargo.toml").exists());
    assert!(temp.path().join("test-project/src/main.rs").exists());
}
```


## Cursor rule: `.cursor/rules/cloud-integrations.mdc`

# Cloud Provider Integrations

Guidelines for developing cloud provider integrations in Armature.

## Cloud Crates Overview

| Crate | Provider | Services |
|-------|----------|----------|
| `armature-aws` | AWS | S3, DynamoDB, SQS, SNS, SES, Lambda, KMS, Cognito |
| `armature-gcp` | GCP | Cloud Storage, Pub/Sub, Firestore, Spanner, BigQuery |
| `armature-azure` | Azure | Blob Storage, Cosmos DB, Service Bus, Key Vault |
| `armature-redis` | Redis | Connection pooling, Pub/Sub, Cluster |
| `armature-lambda` | AWS Lambda | Lambda runtime integration |
| `armature-cloudrun` | Cloud Run | Cloud Run runtime integration |
| `armature-azure-functions` | Azure Functions | Azure Functions runtime |

## Architecture Pattern

### Feature-Gated Services

Each cloud crate uses feature flags to enable only needed services:

```toml
# armature-aws/Cargo.toml
[features]
default = []
full = ["s3", "dynamodb", "sqs", "sns", "ses", "lambda", "kms", "cognito"]
s3 = ["aws-sdk-s3"]
dynamodb = ["aws-sdk-dynamodb"]
sqs = ["aws-sdk-sqs"]
sns = ["aws-sdk-sns"]
ses = ["aws-sdk-ses"]
lambda = ["aws-sdk-lambda"]
kms = ["aws-sdk-kms"]
cognito = ["aws-sdk-cognitoidentityprovider"]
```

### Service Factory Pattern

```rust
// armature-aws/src/lib.rs
pub struct AwsServices {
    config: AwsConfig,
    #[cfg(feature = "s3")]
    s3: OnceCell<S3Client>,
    #[cfg(feature = "dynamodb")]
    dynamodb: OnceCell<DynamoDbClient>,
    // ... other services
}

impl AwsServices {
    pub async fn new(config: AwsConfig) -> Result<Self, AwsError> {
        Ok(Self {
            config,
            #[cfg(feature = "s3")]
            s3: OnceCell::new(),
            #[cfg(feature = "dynamodb")]
            dynamodb: OnceCell::new(),
        })
    }

    #[cfg(feature = "s3")]
    pub fn s3(&self) -> Result<&S3Client, AwsError> {
        self.s3.get_or_try_init(|| {
            S3Client::new(&self.config.sdk_config)
        })
    }

    #[cfg(feature = "dynamodb")]
    pub fn dynamodb(&self) -> Result<&DynamoDbClient, AwsError> {
        self.dynamodb.get_or_try_init(|| {
            DynamoDbClient::new(&self.config.sdk_config)
        })
    }
}
```

### Configuration from Environment

```rust
// armature-aws/src/config.rs
#[derive(Debug, Clone)]
pub struct AwsConfig {
    pub region: String,
    pub sdk_config: SdkConfig,
    enabled_services: HashSet<String>,
}

impl AwsConfig {
    pub fn from_env() -> AwsConfigBuilder {
        AwsConfigBuilder {
            region: std::env::var("AWS_REGION").ok(),
            profile: std::env::var("AWS_PROFILE").ok(),
            endpoint_url: std::env::var("AWS_ENDPOINT_URL").ok(),
            enabled_services: HashSet::new(),
        }
    }

    pub fn builder() -> AwsConfigBuilder {
        AwsConfigBuilder::default()
    }
}

#[derive(Default)]
pub struct AwsConfigBuilder {
    region: Option<String>,
    profile: Option<String>,
    endpoint_url: Option<String>,
    enabled_services: HashSet<String>,
}

impl AwsConfigBuilder {
    pub fn region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    pub fn enable_s3(mut self) -> Self {
        self.enabled_services.insert("s3".to_string());
        self
    }

    pub fn enable_dynamodb(mut self) -> Self {
        self.enabled_services.insert("dynamodb".to_string());
        self
    }

    pub async fn build(self) -> Result<AwsConfig, AwsError> {
        let mut config_loader = aws_config::defaults(BehaviorVersion::latest());

        if let Some(region) = &self.region {
            config_loader = config_loader.region(Region::new(region.clone()));
        }

        if let Some(endpoint) = &self.endpoint_url {
            config_loader = config_loader.endpoint_url(endpoint);
        }

        let sdk_config = config_loader.load().await;

        Ok(AwsConfig {
            region: self.region.unwrap_or_else(|| "us-east-1".to_string()),
            sdk_config,
            enabled_services: self.enabled_services,
        })
    }
}
```

## DI Integration Pattern

```rust
// In user's module
use armature::prelude::*;
use armature_aws::*;

#[module(
    providers: [AwsServicesProvider],
    controllers: [FileController],
)]
struct CloudModule;

// Provider for DI
#[injectable]
pub struct AwsServicesProvider {
    services: Arc<AwsServices>,
}

impl AwsServicesProvider {
    pub async fn new() -> Result<Self, AwsError> {
        let config = AwsConfig::from_env()
            .enable_s3()
            .enable_sqs()
            .build()
            .await?;

        let services = AwsServices::new(config).await?;

        Ok(Self {
            services: Arc::new(services),
        })
    }
}

// Usage in controller
#[controller("/files")]
struct FileController {
    aws: AwsServicesProvider,
}

impl FileController {
    #[post("/upload")]
    async fn upload(&self, body: Bytes) -> Result<Json<UploadResponse>, Error> {
        let s3 = self.aws.services.s3()?;

        s3.put_object()
            .bucket("my-bucket")
            .key("file.txt")
            .body(body.into())
            .send()
            .await?;

        Ok(Json(UploadResponse { success: true }))
    }
}
```

## Error Handling

### Unified Error Type

```rust
// armature-aws/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AwsError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Service not enabled: {0}")]
    ServiceNotEnabled(String),

    #[error("S3 error: {0}")]
    #[cfg(feature = "s3")]
    S3(#[from] aws_sdk_s3::Error),

    #[error("DynamoDB error: {0}")]
    #[cfg(feature = "dynamodb")]
    DynamoDb(#[from] aws_sdk_dynamodb::Error),

    #[error("SQS error: {0}")]
    #[cfg(feature = "sqs")]
    Sqs(#[from] aws_sdk_sqs::Error),

    #[error("SDK error: {0}")]
    Sdk(String),
}

// Convert to HTTP error
impl From<AwsError> for armature_core::Error {
    fn from(err: AwsError) -> Self {
        armature_core::Error::ServiceUnavailable(err.to_string())
    }
}
```

## Testing with LocalStack/Emulators

### LocalStack for AWS

```rust
#[cfg(test)]
mod tests {
    use super::*;

    async fn localstack_config() -> AwsConfig {
        AwsConfig::builder()
            .region("us-east-1")
            .endpoint_url("http://localhost:4566")
            .enable_s3()
            .build()
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_s3_upload() {
        let config = localstack_config().await;
        let aws = AwsServices::new(config).await.unwrap();
        let s3 = aws.s3().unwrap();

        // Create bucket
        s3.create_bucket()
            .bucket("test-bucket")
            .send()
            .await
            .unwrap();

        // Test upload
        s3.put_object()
            .bucket("test-bucket")
            .key("test.txt")
            .body(Bytes::from("hello").into())
            .send()
            .await
            .unwrap();

        // Verify
        let result = s3.get_object()
            .bucket("test-bucket")
            .key("test.txt")
            .send()
            .await
            .unwrap();

        let body = result.body.collect().await.unwrap().into_bytes();
        assert_eq!(body.as_ref(), b"hello");
    }
}
```

### Docker Compose for Testing

```yaml
# docker-compose.test.yml
version: '3.8'
services:
  localstack:
    image: localstack/localstack:latest
    ports:
      - "4566:4566"
    environment:
      - SERVICES=s3,sqs,dynamodb,sns

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"

  gcp-emulator:
    image: gcr.io/google.com/cloudsdktool/google-cloud-cli:emulators
    ports:
      - "8085:8085"
    command: gcloud beta emulators pubsub start --host-port=0.0.0.0:8085
```

## GCP Integration Pattern

```rust
// armature-gcp/src/lib.rs
pub struct GcpServices {
    config: GcpConfig,
    #[cfg(feature = "storage")]
    storage: OnceCell<StorageClient>,
    #[cfg(feature = "pubsub")]
    pubsub: OnceCell<PubSubClient>,
    #[cfg(feature = "firestore")]
    firestore: OnceCell<FirestoreClient>,
}

impl GcpServices {
    pub async fn new(config: GcpConfig) -> Result<Self, GcpError> {
        Ok(Self {
            config,
            #[cfg(feature = "storage")]
            storage: OnceCell::new(),
            #[cfg(feature = "pubsub")]
            pubsub: OnceCell::new(),
            #[cfg(feature = "firestore")]
            firestore: OnceCell::new(),
        })
    }

    #[cfg(feature = "storage")]
    pub async fn storage(&self) -> Result<&StorageClient, GcpError> {
        self.storage.get_or_try_init(|| async {
            StorageClient::new(&self.config).await
        }).await
    }
}
```

## Azure Integration Pattern

```rust
// armature-azure/src/lib.rs
pub struct AzureServices {
    config: AzureConfig,
    #[cfg(feature = "blob")]
    blob: OnceCell<BlobServiceClient>,
    #[cfg(feature = "cosmos")]
    cosmos: OnceCell<CosmosClient>,
    #[cfg(feature = "servicebus")]
    servicebus: OnceCell<ServiceBusClient>,
}

impl AzureServices {
    pub async fn new(config: AzureConfig) -> Result<Self, AzureError> {
        Ok(Self {
            config,
            #[cfg(feature = "blob")]
            blob: OnceCell::new(),
            #[cfg(feature = "cosmos")]
            cosmos: OnceCell::new(),
            #[cfg(feature = "servicebus")]
            servicebus: OnceCell::new(),
        })
    }
}
```

## Redis Integration

```rust
// armature-redis/src/lib.rs
use deadpool_redis::{Config, Pool, Runtime};

pub struct RedisService {
    pool: Pool,
}

impl RedisService {
    pub async fn new(config: RedisConfig) -> Result<Self, RedisError> {
        let cfg = Config::from_url(&config.url);
        let pool = cfg.create_pool(Some(Runtime::Tokio1))?;

        Ok(Self { pool })
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>, RedisError> {
        let mut conn = self.pool.get().await?;
        let result: Option<String> = redis::cmd("GET")
            .arg(key)
            .query_async(&mut conn)
            .await?;
        Ok(result)
    }

    pub async fn set(&self, key: &str, value: &str, ttl: Option<Duration>) -> Result<(), RedisError> {
        let mut conn = self.pool.get().await?;
        let mut cmd = redis::cmd("SET");
        cmd.arg(key).arg(value);

        if let Some(ttl) = ttl {
            cmd.arg("EX").arg(ttl.as_secs());
        }

        cmd.query_async(&mut conn).await?;
        Ok(())
    }

    pub async fn publish(&self, channel: &str, message: &str) -> Result<(), RedisError> {
        let mut conn = self.pool.get().await?;
        redis::cmd("PUBLISH")
            .arg(channel)
            .arg(message)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}
```

## Serverless Runtime Integrations

### AWS Lambda

```rust
// armature-lambda/src/lib.rs
use lambda_runtime::{service_fn, LambdaEvent, Error};

pub async fn run_lambda<H, Req, Res>(handler: H) -> Result<(), Error>
where
    H: Fn(Req) -> Res + Send + Sync + 'static,
    Req: DeserializeOwned,
    Res: Future<Output = Result<Response, Error>> + Send,
{
    lambda_runtime::run(service_fn(|event: LambdaEvent<Req>| async {
        handler(event.payload).await
    })).await
}
```

### Cloud Run

```rust
// armature-cloudrun/src/lib.rs
pub fn cloud_run_port() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080)
}

pub async fn run_cloud_run<M: Module>(module: M) -> Result<(), Error> {
    let app = Application::create(module);
    let port = cloud_run_port();
    app.listen(port).await
}
```

## Summary

1. Use **feature flags** to enable only needed services
2. Implement **lazy initialization** with `OnceCell`
3. Support **environment-based configuration**
4. Provide **unified error types** that convert to HTTP errors
5. Include **DI integration** for seamless injection
6. Use **local emulators** (LocalStack, etc.) for testing
7. Follow consistent patterns across all cloud providers


## Cursor rule: `.cursor/rules/database-integration.mdc`

_Database integration patterns for Armature applications_

Applies to: `**/repositories/**/*.rs, **/models/**/*.rs, **/migrations/**/*.rs`

# Database Integration

Guidelines for database integration with Diesel or SeaORM.

## Connection Pooling

```rust
use diesel::r2d2::{ConnectionManager, Pool};
use diesel::PgConnection;

pub type DbPool = Pool<ConnectionManager<PgConnection>>;

pub fn create_pool(database_url: &str) -> DbPool {
    let manager = ConnectionManager::<PgConnection>::new(database_url);
    Pool::builder()
        .max_size(10)
        .min_idle(Some(2))
        .connection_timeout(Duration::from_secs(5))
        .build(manager)
        .expect("Failed to create pool")
}
```

## Repository Pattern

```rust
#[injectable]
pub struct UserRepository {
    pool: Arc<DbPool>,
}

impl UserRepository {
    pub async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, DbError> {
        let conn = self.pool.get()?;

        users::table
            .find(id)
            .first(&conn)
            .optional()
            .map_err(Into::into)
    }

    pub async fn create(&self, new_user: NewUser) -> Result<User, DbError> {
        let conn = self.pool.get()?;

        diesel::insert_into(users::table)
            .values(&new_user)
            .get_result(&conn)
            .map_err(Into::into)
    }
}
```

## Transactions

```rust
pub async fn transfer_funds(
    &self,
    from: Uuid,
    to: Uuid,
    amount: Decimal,
) -> Result<(), DbError> {
    let conn = self.pool.get()?;

    conn.transaction(|conn| {
        // Debit source account
        diesel::update(accounts::table.find(from))
            .set(accounts::balance.eq(accounts::balance - amount))
            .execute(conn)?;

        // Credit destination account
        diesel::update(accounts::table.find(to))
            .set(accounts::balance.eq(accounts::balance + amount))
            .execute(conn)?;

        Ok(())
    })
}
```

## Query Optimization

```rust
// Select only needed columns
users::table
    .select((users::id, users::name, users::email))
    .load::<(Uuid, String, String)>(&conn)?;

// Use joins instead of N+1 queries
users::table
    .inner_join(posts::table)
    .filter(users::id.eq(user_id))
    .select((users::all_columns, posts::all_columns))
    .load::<(User, Post)>(&conn)?;

// Paginate large result sets
users::table
    .order(users::created_at.desc())
    .limit(20)
    .offset(page * 20)
    .load::<User>(&conn)?;
```

## Model Definitions

```rust
use diesel::prelude::*;

#[derive(Queryable, Identifiable, Selectable)]
#[diesel(table_name = users)]
pub struct User {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Insertable)]
#[diesel(table_name = users)]
pub struct NewUser {
    pub name: String,
    pub email: String,
    pub password_hash: String,
}

#[derive(AsChangeset)]
#[diesel(table_name = users)]
pub struct UpdateUser {
    pub name: Option<String>,
    pub email: Option<String>,
}
```

## Migrations

```bash
# Create migration
diesel migration generate create_users

# Run migrations
diesel migration run

# Revert last migration
diesel migration revert
```

Migration file structure:

```sql
-- up.sql
CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL,
    email VARCHAR(255) NOT NULL UNIQUE,
    password_hash VARCHAR(255) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_email ON users(email);

-- down.sql
DROP TABLE users;
```

## Testing with Transactions

```rust
#[tokio::test]
async fn test_user_creation() {
    let pool = create_test_pool();
    let conn = pool.get().unwrap();

    // Wrap test in transaction that rolls back
    conn.test_transaction(|conn| {
        let repo = UserRepository::new(pool.clone());

        let user = repo.create(NewUser {
            name: "Test".into(),
            email: "test@example.com".into(),
            password_hash: "hash".into(),
        })?;

        assert_eq!(user.name, "Test");
        Ok(())
    });
}
```

## Error Handling

```rust
#[derive(Error, Debug)]
pub enum DbError {
    #[error("Record not found")]
    NotFound,

    #[error("Duplicate key: {0}")]
    Duplicate(String),

    #[error("Connection error: {0}")]
    Connection(#[from] r2d2::Error),

    #[error("Query error: {0}")]
    Query(#[from] diesel::result::Error),
}
```


## Cursor rule: `.cursor/rules/dependency-injection.mdc`

_Dependency injection patterns for Armature applications_

Applies to: `armature-di/**/*.rs, "**/providers/**/*.rs, **/modules/**/*.rs`

# Dependency Injection

Guidelines for using Armature's dependency injection system.

## Injectable Services

```rust
use armature_di::injectable;

#[injectable]
pub struct UserService {
    repository: Arc<dyn UserRepository>,
    cache: Arc<dyn Cache>,
}

impl UserService {
    pub fn new(
        repository: Arc<dyn UserRepository>,
        cache: Arc<dyn Cache>,
    ) -> Self {
        Self { repository, cache }
    }

    pub async fn get_user(&self, id: Uuid) -> Result<User, Error> {
        // Check cache first
        if let Some(user) = self.cache.get(&format!("user:{}", id)).await? {
            return Ok(user);
        }

        // Fetch from repository
        let user = self.repository.find_by_id(id).await?;

        // Cache for future requests
        self.cache.set(&format!("user:{}", id), &user, Duration::from_secs(300)).await?;

        Ok(user)
    }
}
```

## Module Configuration

```rust
use armature_di::module;

#[module]
pub struct AppModule {
    #[provider]
    user_service: UserService,

    #[provider]
    post_service: PostService,

    #[controller]
    users_controller: UsersController,

    #[controller]
    posts_controller: PostsController,
}

impl AppModule {
    pub fn new(config: &Config) -> Self {
        // Configure module with dependencies
    }
}
```

## Provider Scopes

```rust
// Singleton - one instance for entire application
#[injectable(scope = "singleton")]
pub struct DatabasePool { }

// Request - new instance per HTTP request
#[injectable(scope = "request")]
pub struct RequestContext { }

// Transient - new instance every time (default)
#[injectable]
pub struct EmailSender { }
```

## Factory Providers

```rust
#[module]
pub struct DatabaseModule;

impl DatabaseModule {
    #[provider]
    fn provide_pool(config: &DatabaseConfig) -> DbPool {
        create_pool(&config.url)
    }

    #[provider]
    fn provide_user_repo(pool: Arc<DbPool>) -> Arc<dyn UserRepository> {
        Arc::new(PgUserRepository::new(pool))
    }
}
```

## Interface Binding

```rust
// Define trait
pub trait EmailSender: Send + Sync {
    async fn send(&self, email: &Email) -> Result<(), Error>;
}

// Implement for production
#[injectable(provides = "dyn EmailSender")]
pub struct SmtpEmailSender {
    config: SmtpConfig,
}

// Implement for testing
pub struct MockEmailSender {
    sent: Arc<Mutex<Vec<Email>>>,
}

// Register in module
#[module]
pub struct MailModule;

impl MailModule {
    #[provider]
    fn provide_email_sender(config: &Config) -> Arc<dyn EmailSender> {
        if config.is_test() {
            Arc::new(MockEmailSender::new())
        } else {
            Arc::new(SmtpEmailSender::new(&config.smtp))
        }
    }
}
```

## Resolving Dependencies

```rust
// In controllers - automatic injection
#[controller("/users")]
pub struct UsersController {
    user_service: Arc<UserService>, // Automatically injected
}

// Manual resolution
let user_service = container.resolve::<UserService>();
let email_sender = container.resolve::<dyn EmailSender>();
```

## Lifecycle Hooks

```rust
#[injectable]
pub struct CacheService {
    client: RedisClient,
}

impl OnInit for CacheService {
    async fn on_init(&self) -> Result<(), Error> {
        // Run after construction
        self.client.ping().await?;
        tracing::info!("Cache service initialized");
        Ok(())
    }
}

impl OnDestroy for CacheService {
    async fn on_destroy(&self) {
        // Cleanup before shutdown
        self.client.close().await;
        tracing::info!("Cache service shutdown");
    }
}
```

## Testing with DI

```rust
#[tokio::test]
async fn test_user_service() {
    // Create test container with mocks
    let container = Container::test()
        .with::<dyn UserRepository>(Arc::new(MockUserRepository::new()))
        .with::<dyn Cache>(Arc::new(MockCache::new()))
        .build();

    let service = container.resolve::<UserService>();

    let result = service.get_user(Uuid::new_v4()).await;
    assert!(result.is_ok());
}
```

## Circular Dependency Prevention

```rust
// Bad - circular dependency
#[injectable]
pub struct ServiceA {
    b: Arc<ServiceB>, // ServiceB also depends on ServiceA
}

// Good - use lazy resolution or events
#[injectable]
pub struct ServiceA {
    container: Arc<Container>,
}

impl ServiceA {
    fn get_b(&self) -> Arc<ServiceB> {
        self.container.resolve::<ServiceB>()
    }
}
```

## Best Practices

1. **Depend on abstractions** - Use `Arc<dyn Trait>` over concrete types
2. **Constructor injection** - Prefer constructor over field injection
3. **Small interfaces** - Keep traits focused (ISP)
4. **Avoid service locator** - Don't pass Container everywhere
5. **Test with mocks** - Use trait bounds for testability


## Cursor rule: `.cursor/rules/docs-website-components.mdc`

# Documentation Website Component Structure

## Rule

When creating or modifying Angular components for the documentation website (`web/src/app/pages/docs/`), **always use external HTML template files** instead of inline templates.

## Structure

Each documentation page component should have the following file structure:

```
web/src/app/pages/docs/pages/<component-name>/
├── <component-name>.component.ts    # Component class with templateUrl
├── <component-name>.component.html  # External HTML template
└── <component-name>.component.scss  # Optional: Component-specific styles
```

## Component TypeScript Pattern

```typescript
import { Component } from '@angular/core';
import { CommonModule } from '@angular/common';
import { RouterModule } from '@angular/router';

@Component({
  selector: 'app-<component-name>',
  standalone: true,
  imports: [CommonModule, RouterModule],
  templateUrl: './<component-name>.component.html',
  styleUrls: ['./<component-name>.component.scss']  // Optional
})
export class <ComponentName>Component {
  // Component logic
}
```

## Why This Rule Exists

1. **Readability** - HTML templates for documentation pages can be large; separating them improves maintainability
2. **Editor Support** - External HTML files get better syntax highlighting and IntelliSense
3. **Consistency** - All documentation components follow the same pattern
4. **Diffing** - Easier to review changes when HTML is in separate files
5. **Hot Reload** - Some Angular tooling handles external templates better for HMR

## Exceptions

- Small utility components with minimal templates (< 10 lines) may use inline templates
- The `DocPageComponent` shared component uses inline template as it's a wrapper

## Examples

### ✅ Good - External Template

```typescript
// grafana-dashboards.component.ts
@Component({
  selector: 'app-grafana-dashboards',
  standalone: true,
  imports: [CommonModule, RouterModule],
  templateUrl: './grafana-dashboards.component.html',
  styleUrls: ['./grafana-dashboards.component.scss']
})
export class GrafanaDashboardsComponent {
  // ...
}
```

### ❌ Bad - Inline Template (for doc pages)

```typescript
// grafana-dashboards.component.ts
@Component({
  selector: 'app-grafana-dashboards',
  standalone: true,
  imports: [CommonModule, RouterModule],
  template: `
    <div class="doc-content">
      <!-- Large HTML content here -->
    </div>
  `
})
export class GrafanaDashboardsComponent {
  // ...
}
```

## Shared Styles

Documentation components should import the shared documentation styles:

```scss
// <component-name>.component.scss
@use '../../_doc-content' as doc;

// Component-specific styles here
```

## Adding New Documentation Pages

When adding a new documentation page:

1. Create the component directory: `web/src/app/pages/docs/pages/<name>/`
2. Create `<name>.component.ts` with `templateUrl`
3. Create `<name>.component.html` with the template
4. Create `<name>.component.scss` importing shared styles
5. Register in `docs.component.ts`:
   - Import the component
   - Add to `imports` array
   - Add to `docs` array with `hasComponent: true`
   - Add `@case` in the template switch


## Cursor rule: `.cursor/rules/documentation-naming.mdc`

# Documentation Naming Convention

## Rule
All documentation files must follow these conventions:

### File Naming
- Use **UPPER_SNAKE_CASE** for all documentation file names
- Example: `SECURITY_GUIDE.md`, `API_VERSIONING_GUIDE.md`, `ERROR_CORRELATION.md`

### Location
- All documentation files must be placed in the `docs/` folder
- Website copies should be synced to `web/public/docs/`

### Examples

**Correct:**
```
docs/GETTING_STARTED.md
docs/API_REFERENCE.md
docs/SECURITY_GUIDE.md
docs/ERROR_CORRELATION_GUIDE.md
docs/REQUEST_EXTRACTORS.md
```

**Incorrect:**
```
docs/getting-started.md          # lowercase with hyphens
docs/apiReference.md             # camelCase
docs/security_guide.md           # lowercase
README.md                        # should be in docs/ if it's documentation
src/docs/guide.md               # wrong location
```

### Exceptions
- `README.md` at project root is acceptable
- `CHANGELOG.md` at project root is acceptable
- `CONTRIBUTING.md` at project root is acceptable
- `LICENSE` at project root is acceptable

### When Creating Documentation
1. Name the file in UPPER_SNAKE_CASE with `.md` extension
2. Place it in the `docs/` directory
3. If needed for the website, also copy to `web/public/docs/`


## Cursor rule: `.cursor/rules/documentation-sync.mdc`

# Documentation Sync Rule

## Rule

Whenever documentation files are added or modified in the `docs/` directory, the corresponding website documentation in `web/` must also be updated.

## Background

The Armature project has two documentation locations:

1. **Source Documentation**: `docs/` directory - Markdown files for the framework
2. **Website Documentation**: `web/src/app/pages/docs/` - Angular components that display the documentation

The website reads markdown files from `web/public/docs/` (which is symlinked to `docs/` in development).

## Required Actions

### When Adding New Documentation

1. Create the markdown file in `docs/` following the `UPPER_SNAKE_CASE.md` naming convention
2. Update `web/src/app/pages/docs/docs.component.ts` to include the new document in the documentation list:

```typescript
// Add to the docs array in DocsComponent
{
  title: 'Your New Guide',
  filename: 'YOUR_NEW_GUIDE.md',
  description: 'Brief description of what this guide covers'
}
```

3. Optionally update the website navigation if the document should appear in the main menu

### When Modifying Existing Documentation

- Changes to existing `docs/*.md` files are automatically reflected on the website due to the symlink
- No additional action needed unless renaming files

### When Renaming Documentation

1. Rename the file in `docs/` to the new `UPPER_SNAKE_CASE.md` name
2. Update the `filename` field in `web/src/app/pages/docs/docs.component.ts`

### When Deleting Documentation

1. Remove the file from `docs/`
2. Remove the corresponding entry from `web/src/app/pages/docs/docs.component.ts`

## File Locations

| Purpose | Location |
|---------|----------|
| Source docs | `docs/*.md` |
| Docs component | `web/src/app/pages/docs/docs.component.ts` |
| Docs styles | `web/src/app/pages/docs/docs.component.scss` |
| Public symlink | `web/public/docs/` → `../../docs/` |

## Example

When adding a new guide called "Request Validation Guide":

1. Create `docs/REQUEST_VALIDATION_GUIDE.md`
2. Update `docs.component.ts`:

```typescript
docs = [
  // ... existing docs ...
  {
    title: 'Request Validation Guide',
    filename: 'REQUEST_VALIDATION_GUIDE.md',
    description: 'How to validate incoming requests'
  },
];
```

## CI/CD Note

The GitHub Actions workflow (`.github/workflows/docs.yml`) automatically copies all `docs/*.md` files to the deployed website during the build process. No manual copy step is needed for deployment.


## Cursor rule: `.cursor/rules/documentation.mdc`

# Documentation Standards

All documentation for the Armature project must be generated in the `docs/` directory following these standards.

## Directory Structure

All documentation files go directly in the `docs/` root directory. **Do NOT create subfolders.**

```
docs/
├── README.md                    # Documentation index
├── getting-started.md           # Getting started guide
├── auth-guide.md                # Authentication guide
├── cache-guide.md               # Cache guide
├── cron-guide.md                # Cron guide
├── queue-guide.md               # Queue guide
├── deployment-guide.md          # Deployment guide
└── *.md                         # All other documentation
```

**Important:** Do NOT create subdirectories like `guides/`, `modules/`, or `examples/`. All `.md` files belong in `docs/` root.

## File Naming

- Use **lowercase with hyphens**: `my-feature-guide.md`
- Be **descriptive**: `oauth2-providers-guide.md` not `oauth.md`
- Use **.md extension** for all Markdown files
- Avoid abbreviations unless widely understood

### Good Examples ✅

- `websocket-sse-guide.md`
- `authentication-guide.md`
- `rate-limiting-configuration.md`

### Bad Examples ❌

- `WS_SSE.md` (uppercase, abbreviation)
- `auth.md` (too generic)
- `guide-1.md` (not descriptive)

## Documentation Requirements

### Every Feature Must Have Documentation

When adding a new feature, you MUST create corresponding documentation in `docs/`:

1. **Feature Guide** (`docs/<feature>-guide.md`)
   - Overview of the feature
   - Key concepts
   - Configuration options
   - Step-by-step instructions
   - Code examples
   - Best practices
   - Troubleshooting

2. **API Reference** (inline code docs)
   - Rust doc comments (`///`)
   - Examples in doc comments
   - Clear parameter descriptions

3. **Code Examples** (`examples/` directory - code only)
   - Working code example
   - Comments explaining key parts
   - README if complex

## Documentation Format

### Markdown Structure

```markdown
# Title

Brief one-paragraph introduction.

## Table of Contents

- [Section 1](#section-1)
- [Section 2](#section-2)

## Overview

High-level explanation of what this is and why it exists.

## Features

- ✅ Feature 1
- ✅ Feature 2
- ✅ Feature 3

## Usage

### Basic Example

\`\`\`rust
// Working code example
use armature::prelude::*;

#[tokio::main]
async fn main() {
    // Example code
}
\`\`\`

### Advanced Example

More complex usage...

## Configuration

Detailed configuration options...

## Best Practices

1. Practice one
2. Practice two

## Common Pitfalls

- ❌ Don't do this
- ✅ Do this instead

## API Reference

Link to generated API docs or inline reference.

## Summary

Quick recap of key points.
```

### Required Sections

Every guide must include:

1. **Title** - Clear, descriptive
2. **Overview** - What and why
3. **Features** - Bullet list of capabilities
4. **Usage** - At least one working example
5. **Best Practices** - Dos and don'ts
6. **Summary** - Quick reference

## Code Examples

### Requirements

- **Must be runnable** without errors
- **Include necessary imports**
- **Add comments** for non-obvious code
- **Use realistic scenarios**
- **Show error handling**

### Good Example ✅

```rust
use armature_queue::*;

#[tokio::main]
async fn main() -> Result<(), QueueError> {
    // Connect to Redis
    let queue = Queue::new("redis://localhost:6379", "default").await?;

    // Enqueue a job
    let job_id = queue.enqueue(
        "send_email",
        serde_json::json!({
            "to": "user@example.com",
            "subject": "Welcome!"
        })
    ).await?;

    println!("Job enqueued: {}", job_id);
    Ok(())
}
```

### Bad Example ❌

```rust
// Incomplete, won't compile
let queue = Queue::new("redis://localhost:6379");
queue.enqueue("send_email", data);
```

## Inline Code Documentation

### Rust Doc Comments

```rust
/// Brief one-line description.
///
/// More detailed explanation of what this does,
/// including any important details.
///
/// # Arguments
///
/// * `param1` - Description of param1
/// * `param2` - Description of param2
///
/// # Returns
///
/// What this function returns and when.
///
/// # Errors
///
/// Possible error conditions and what causes them.
///
/// # Examples
///
/// ```
/// use armature_queue::Queue;
///
/// # async fn example() -> Result<(), QueueError> {
/// let queue = Queue::new("redis://localhost:6379", "default").await?;
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Conditions under which this panics (if any).
pub async fn example_function(param1: String, param2: i32) -> Result<String, Error> {
    // Implementation
}
```

### Module-Level Documentation

```rust
//! Job queue module for background processing.
//!
//! This module provides a Redis-backed job queue system with
//! automatic retries, priorities, and scheduled jobs.
//!
//! # Examples
//!
//! ```no_run
//! use armature_queue::*;
//!
//! # async fn example() -> Result<(), QueueError> {
//! let queue = Queue::new("redis://localhost:6379", "default").await?;
//! queue.enqueue("task", serde_json::json!({})).await?;
//! # Ok(())
//! # }
//! ```
```

## Documentation Types

### 1. Getting Started Guides

**Purpose:** Help new users quickly start using the feature.

**Structure:**
- Prerequisites
- Installation
- Quick start (5 minutes or less)
- Next steps

### 2. Concept Guides

**Purpose:** Explain concepts and architecture.

**Structure:**
- What is this?
- Why does it exist?
- How does it work?
- When to use it?

### 3. How-To Guides

**Purpose:** Step-by-step instructions for specific tasks.

**Structure:**
- Problem statement
- Prerequisites
- Step-by-step solution
- Verification
- Troubleshooting

### 4. Reference Documentation

**Purpose:** Complete technical details.

**Structure:**
- All configuration options
- All API methods
- All types and enums
- All error codes

## Visual Aids

### Use Diagrams When Helpful

```markdown
## Architecture

\`\`\`
┌──────────┐     ┌──────────┐     ┌──────────┐
│  Client  │────▶│  Server  │────▶│ Database │
└──────────┘     └──────────┘     └──────────┘
\`\`\`
```

### Use Tables for Comparisons

```markdown
| Feature | Option A | Option B |
|---------|----------|----------|
| Speed   | Fast     | Slow     |
| Memory  | Low      | High     |
```

### Use Lists for Steps

```markdown
1. First step
2. Second step
3. Third step
```

## Keeping Documentation Updated

### When Code Changes, Update Docs

**CRITICAL:** Documentation must be updated in the same PR as code changes.

### Update Checklist

- [ ] Inline code comments updated
- [ ] API reference updated
- [ ] User guide updated (if behavior changed)
- [ ] Examples updated (if API changed)
- [ ] README updated (if new feature)
- [ ] CHANGELOG.md updated

### Documentation Review

Before merging:
1. All code examples compile
2. All links work
3. No typos or grammar errors
4. Formatting is consistent
5. Screenshots are up to date (if any)

## Linking

### Internal Links

```markdown
See the [Authentication Guide](./AUTH_GUIDE.md) for details.

Jump to [Configuration](#configuration) section.
```

### External Links

```markdown
See the [Rust Book](https://doc.rust-lang.org/book/) for more information.
```

### API Links

```markdown
See [`Queue::enqueue`](../armature-queue/src/queue.rs) for details.
```

## Common Patterns

### Feature Documentation Template

```markdown
# Feature Name

Brief description of what this feature does.

## Features

- ✅ Feature 1
- ✅ Feature 2

## Basic Usage

\`\`\`rust
// Working example
\`\`\`

## Configuration

### Options

- `option1` - Description
- `option2` - Description

## Examples

### Example 1: Common Use Case

\`\`\`rust
// Example code
\`\`\`

### Example 2: Advanced Use Case

\`\`\`rust
// Example code
\`\`\`

## Best Practices

1. Do this
2. Don't do that

## Troubleshooting

### Problem 1

**Symptom:** What you see

**Cause:** Why it happens

**Solution:** How to fix

## API Reference

Link to generated docs or detailed API listing.

## Summary

Key takeaways.
```

## Documentation Testing

### Test Code Examples

```bash
# Test all doc examples
cargo test --doc --all-features

# Test specific module docs
cargo test --doc --package armature-queue
```

### Check Links

```bash
# Use markdown link checker (install if needed)
markdown-link-check docs/**/*.md
```

## Documentation Tools

### Generate API Docs

```bash
# Generate and open API documentation
cargo doc --all-features --no-deps --open
```

### Build Documentation Site

```bash
# If using mdBook or similar
mdbook build docs/
mdbook serve docs/
```

## Exception: Root README

The **root README.md** stays in the project root directory, not in `docs/`.

```
armature/
├── README.md          ← Project overview (root)
├── docs/
│   ├── README.md      ← Documentation index
│   └── *.md           ← All other docs (flat, no subfolders)
└── ...
```

## Documentation Metrics

### Quality Indicators

- ✅ All public APIs have doc comments
- ✅ All features have user guides
- ✅ All code examples compile
- ✅ No broken links
- ✅ No spelling errors
- ✅ Consistent formatting

### Coverage Goals

- **100%** of public API documented
- **100%** of features have guides
- **90%+** of doc examples compile
- **100%** of links work

## Summary

**Key Principles:**

1. **Location:** All docs in `docs/` root directory - NO subfolders (except root README)
2. **Naming:** lowercase-with-hyphens.md
3. **Flat Structure:** All documentation files go directly in `docs/`, not in subdirectories
4. **Completeness:** Every feature must have documentation
5. **Quality:** Working code examples, clear explanations
6. **Maintenance:** Update docs with code changes
7. **Testing:** All examples must compile

**Documentation is code!** Treat it with the same care and rigor. 📚


## Cursor rule: `.cursor/rules/gitflow-branching.mdc`

# Gitflow Branching Strategy

This project follows the **Gitflow** branching model for organized development and release management.

## Branch Structure

### Main Branches (Long-lived)

#### `main`
- **Purpose:** Production-ready code
- **Protected:** Yes
- **Merged from:** `release/*` and `hotfix/*` only
- **Never commit directly to this branch**

#### `develop`
- **Purpose:** Integration branch for features
- **Protected:** Yes
- **Merged from:** `feature/*`, `release/*`, and `hotfix/*`
- **Base for:** All feature branches

### Supporting Branches (Short-lived)

#### `feature/*`
- **Purpose:** New features or enhancements
- **Naming:** `feature/<issue-number>-<short-description>`
- **Examples:**
  - `feature/123-add-websocket-support`
  - `feature/456-user-authentication`
- **Base:** `develop`
- **Merge to:** `develop`
- **Lifetime:** Duration of feature development

#### `release/*`
- **Purpose:** Prepare for production release
- **Naming:** `release/<version>`
- **Examples:**
  - `release/1.0.0`
  - `release/2.1.0`
- **Base:** `develop`
- **Merge to:** `main` and `develop`
- **Lifetime:** Until release is finalized

#### `hotfix/*`
- **Purpose:** Critical bug fixes in production
- **Naming:** `hotfix/<version>-<description>`
- **Examples:**
  - `hotfix/1.0.1-security-patch`
  - `hotfix/2.1.1-memory-leak`
- **Base:** `main`
- **Merge to:** `main` and `develop`
- **Lifetime:** Until hotfix is deployed

## Workflow

### Starting a New Feature

```bash
# Ensure develop is up to date
git checkout develop
git pull origin develop

# Create feature branch
git checkout -b feature/123-add-caching

# Work on feature...
git add .
git commit -m "feat: add Redis caching support"

# Push to remote
git push origin feature/123-add-caching

# Create Pull Request to develop
```

### Completing a Feature

```bash
# Update from develop
git checkout develop
git pull origin develop

git checkout feature/123-add-caching
git merge develop

# Resolve any conflicts
# Run tests
cargo test --all-features

# Push and create PR
git push origin feature/123-add-caching
```

### Creating a Release

```bash
# Create release branch from develop
git checkout develop
git pull origin develop
git checkout -b release/1.0.0

# Update version numbers
# Update CHANGELOG.md
# Final testing

# Commit release preparation
git commit -am "chore: prepare release 1.0.0"

# Merge to main
git checkout main
git merge --no-ff release/1.0.0
git tag -a v1.0.0 -m "Release version 1.0.0"

# Merge back to develop
git checkout develop
git merge --no-ff release/1.0.0

# Push everything
git push origin main develop --tags

# Delete release branch
git branch -d release/1.0.0
git push origin --delete release/1.0.0
```

### Creating a Hotfix

```bash
# Create hotfix branch from main
git checkout main
git pull origin main
git checkout -b hotfix/1.0.1-critical-fix

# Fix the issue
git commit -am "fix: resolve critical security vulnerability"

# Merge to main
git checkout main
git merge --no-ff hotfix/1.0.1-critical-fix
git tag -a v1.0.1 -m "Hotfix version 1.0.1"

# Merge to develop
git checkout develop
git merge --no-ff hotfix/1.0.1-critical-fix

# Push everything
git push origin main develop --tags

# Delete hotfix branch
git branch -d hotfix/1.0.1-critical-fix
git push origin --delete hotfix/1.0.1-critical-fix
```

## Commit Message Convention

Follow **Conventional Commits** specification:

### Format

```
<type>(<scope>): <subject>

<body>

<footer>
```

### Types

- **feat**: New feature
- **fix**: Bug fix
- **docs**: Documentation only changes
- **style**: Code style changes (formatting, missing semicolons, etc.)
- **refactor**: Code refactoring
- **perf**: Performance improvements
- **test**: Adding or updating tests
- **chore**: Maintenance tasks, dependency updates
- **ci**: CI/CD changes
- **build**: Build system changes

### Examples

```bash
# Feature
git commit -m "feat(queue): add job retry with exponential backoff"

# Bug fix
git commit -m "fix(auth): resolve JWT token expiration issue"

# Documentation
git commit -m "docs(readme): update installation instructions"

# Breaking change
git commit -m "feat(api)!: change response format

BREAKING CHANGE: API responses now use camelCase instead of snake_case"

# Multiple changes
git commit -m "chore: update dependencies and fix linting issues

- Update tokio to 1.35
- Update serde to 1.0.195
- Fix clippy warnings in cache module"
```

## Pull Request Guidelines

### Creating PRs

1. **Base branch:**
   - Features → `develop`
   - Hotfixes → `main` (then merge to `develop`)
   - Releases → `main` (then merge to `develop`)

2. **Title format:**
   - Follow commit message convention
   - Example: `feat: add WebSocket support for real-time updates`

3. **Description must include:**
   - Summary of changes
   - Related issue numbers
   - Testing performed
   - Breaking changes (if any)

### PR Template

```markdown
## Description
Brief description of what this PR does

## Related Issues
Closes #123
Relates to #456

## Type of Change
- [ ] Bug fix (non-breaking change which fixes an issue)
- [ ] New feature (non-breaking change which adds functionality)
- [ ] Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] Documentation update

## Testing
- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Manual testing completed

## Checklist
- [ ] Code follows project style guidelines
- [ ] Self-review completed
- [ ] Documentation updated
- [ ] Tests added/updated
- [ ] No new warnings
- [ ] Changelog updated (if applicable)
```

### Review Process

1. **All PRs require:**
   - At least one approval
   - All CI checks passing
   - No merge conflicts

2. **Before merging:**
   - Rebase on target branch if needed
   - Squash commits if too granular
   - Use merge commit (no fast-forward)

## Version Numbers

Follow **Semantic Versioning** (SemVer):

```
MAJOR.MINOR.PATCH

1.0.0
│ │ │
│ │ └─ PATCH: Bug fixes, security patches
│ └─── MINOR: New features, backward-compatible
└───── MAJOR: Breaking changes
```

### Examples

- `1.0.0` → Initial release
- `1.0.1` → Bug fix
- `1.1.0` → New feature
- `2.0.0` → Breaking change

## Branch Protection Rules

### `main` branch

- ✅ Require pull request reviews before merging
- ✅ Require status checks to pass
- ✅ Require branches to be up to date
- ✅ Require conversation resolution
- ✅ Require signed commits
- ❌ Allow force pushes
- ❌ Allow deletions

### `develop` branch

- ✅ Require pull request reviews before merging
- ✅ Require status checks to pass
- ✅ Require branches to be up to date
- ❌ Allow force pushes
- ❌ Allow deletions

## Tagging Strategy

### Tag Format

```
v<MAJOR>.<MINOR>.<PATCH>[-<prerelease>]
```

### Examples

```bash
# Stable releases
v1.0.0
v1.1.0
v2.0.0

# Pre-releases
v1.0.0-alpha.1
v1.0.0-beta.1
v1.0.0-rc.1
```

### Creating Tags

```bash
# Annotated tag (preferred)
git tag -a v1.0.0 -m "Release version 1.0.0"

# Push tag
git push origin v1.0.0

# Push all tags
git push origin --tags
```

## Best Practices

### Do's ✅

- **Keep branches up to date** with their base branch
- **Write descriptive commit messages**
- **Rebase feature branches** before creating PR
- **Delete branches** after merging
- **Test thoroughly** before creating PR
- **Keep PRs focused** on single feature/fix
- **Update documentation** with code changes
- **Use draft PRs** for work in progress

### Don'ts ❌

- **Never force push** to `main` or `develop`
- **Never commit directly** to `main` or `develop`
- **Don't merge without review** (except hotfixes in emergencies)
- **Don't leave stale branches** unmerged
- **Don't mix features** in single branch
- **Don't commit unfinished work** to shared branches
- **Don't ignore merge conflicts**
- **Don't skip CI checks**

## Emergency Procedures

### Critical Production Bug

1. Create hotfix branch from `main`
2. Fix the issue
3. Fast-track PR review
4. Merge to `main` and `develop`
5. Deploy immediately
6. Create post-mortem document

### Reverting Changes

```bash
# Revert a commit
git revert <commit-hash>

# Revert a merge
git revert -m 1 <merge-commit-hash>

# Push the revert
git push origin <branch>
```

## Branch Cleanup

### Local Cleanup

```bash
# Delete merged local branches
git branch --merged | grep -v "\*\|main\|develop" | xargs -n 1 git branch -d

# Delete remote-tracking branches that no longer exist
git fetch --prune
```

### Remote Cleanup

```bash
# Delete remote branch
git push origin --delete feature/old-feature

# Delete multiple remote branches
git branch -r --merged | grep -v main | grep -v develop | sed 's/origin\///' | xargs -n 1 git push origin --delete
```

## Continuous Integration

### Required Checks

- ✅ All tests pass (`cargo test --all-features`)
- ✅ No clippy warnings (`cargo clippy -- -D warnings`)
- ✅ Code is formatted (`cargo fmt -- --check`)
- ✅ Documentation builds (`cargo doc --no-deps`)
- ✅ Minimum 85% code coverage
- ✅ No security vulnerabilities (`cargo audit`)

## Summary

| Branch Type | Base | Merge To | Naming |
|------------|------|----------|--------|
| `feature/*` | `develop` | `develop` | `feature/<issue>-<desc>` |
| `release/*` | `develop` | `main`, `develop` | `release/<version>` |
| `hotfix/*` | `main` | `main`, `develop` | `hotfix/<version>-<desc>` |

**Key Principle:** Keep `main` production-ready at all times! 🚀


## Cursor rule: `.cursor/rules/graphql-development.mdc`

# GraphQL Development

Guidelines for GraphQL API development with Armature.

## GraphQL Crates

| Crate | Purpose |
|-------|---------|
| `armature-graphql` | GraphQL server with async-graphql |
| `armature-graphql-client` | GraphQL client for external APIs |

## Schema Definition

### Types and Objects

```rust
use async_graphql::*;

/// A user in the system.
#[derive(SimpleObject)]
pub struct User {
    /// Unique identifier
    pub id: ID,

    /// User's email address
    pub email: String,

    /// Display name
    pub name: String,

    /// Account creation timestamp
    pub created_at: DateTime<Utc>,
}

/// Extended user type with relations
#[derive(Default)]
pub struct UserType {
    pub user: User,
}

#[Object]
impl UserType {
    async fn id(&self) -> &ID {
        &self.user.id
    }

    async fn email(&self) -> &str {
        &self.user.email
    }

    async fn name(&self) -> &str {
        &self.user.name
    }

    /// User's posts (resolved lazily)
    async fn posts(&self, ctx: &Context<'_>) -> Result<Vec<Post>> {
        let loader = ctx.data::<DataLoader<PostLoader>>()?;
        let posts = loader.load_one(self.user.id.parse()?).await?;
        Ok(posts.unwrap_or_default())
    }

    /// User's role
    async fn role(&self, ctx: &Context<'_>) -> Result<Role> {
        let service = ctx.data::<RoleService>()?;
        service.get_user_role(self.user.id.parse()?).await
    }
}
```

### Input Types

```rust
/// Input for creating a new user.
#[derive(InputObject)]
pub struct CreateUserInput {
    /// User's email (must be unique)
    #[graphql(validator(email))]
    pub email: String,

    /// User's password (min 8 characters)
    #[graphql(validator(min_length = 8))]
    pub password: String,

    /// Display name
    #[graphql(validator(min_length = 1, max_length = 100))]
    pub name: String,
}

/// Input for updating a user.
#[derive(InputObject)]
pub struct UpdateUserInput {
    /// New email address
    #[graphql(validator(email))]
    pub email: Option<String>,

    /// New display name
    #[graphql(validator(min_length = 1, max_length = 100))]
    pub name: Option<String>,
}

/// Filter options for listing users.
#[derive(InputObject, Default)]
pub struct UserFilter {
    /// Filter by name (contains)
    pub name: Option<String>,

    /// Filter by role
    pub role: Option<Role>,

    /// Filter by creation date (after)
    pub created_after: Option<DateTime<Utc>>,
}
```

### Enums

```rust
/// User role in the system.
#[derive(Enum, Copy, Clone, Eq, PartialEq)]
pub enum Role {
    /// Regular user
    User,
    /// Moderator with elevated privileges
    Moderator,
    /// Administrator with full access
    Admin,
}

/// Sort direction.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Default)]
pub enum SortDirection {
    #[default]
    Asc,
    Desc,
}
```

## Query Implementation

```rust
pub struct QueryRoot;

#[Object]
impl QueryRoot {
    /// Get the currently authenticated user.
    async fn me(&self, ctx: &Context<'_>) -> Result<Option<User>> {
        let user = ctx.data_opt::<AuthenticatedUser>();
        match user {
            Some(auth) => {
                let service = ctx.data::<UserService>()?;
                service.find_by_id(auth.user_id).await
            }
            None => Ok(None),
        }
    }

    /// Get a user by ID.
    async fn user(&self, ctx: &Context<'_>, id: ID) -> Result<Option<User>> {
        let service = ctx.data::<UserService>()?;
        service.find_by_id(id.parse()?).await
    }

    /// List users with optional filtering and pagination.
    async fn users(
        &self,
        ctx: &Context<'_>,
        #[graphql(default)] filter: UserFilter,
        #[graphql(default = 1)] page: u32,
        #[graphql(default = 20, validator(maximum = 100))] per_page: u32,
    ) -> Result<UserConnection> {
        let service = ctx.data::<UserService>()?;
        let (users, total) = service.list(filter, page, per_page).await?;

        Ok(UserConnection {
            nodes: users,
            page_info: PageInfo {
                page,
                per_page,
                total,
                has_next_page: (page * per_page) < total as u32,
                has_previous_page: page > 1,
            },
        })
    }

    /// Search users by name or email.
    async fn search_users(
        &self,
        ctx: &Context<'_>,
        query: String,
        #[graphql(default = 10, validator(maximum = 50))] limit: u32,
    ) -> Result<Vec<User>> {
        let service = ctx.data::<UserService>()?;
        service.search(&query, limit).await
    }
}
```

## Mutation Implementation

```rust
pub struct MutationRoot;

#[Object]
impl MutationRoot {
    /// Create a new user account.
    async fn create_user(
        &self,
        ctx: &Context<'_>,
        input: CreateUserInput,
    ) -> Result<User> {
        let service = ctx.data::<UserService>()?;

        // Check for existing email
        if service.email_exists(&input.email).await? {
            return Err(Error::new("Email already registered"));
        }

        service.create(input).await
    }

    /// Update the current user's profile.
    #[graphql(guard = "AuthGuard")]
    async fn update_profile(
        &self,
        ctx: &Context<'_>,
        input: UpdateUserInput,
    ) -> Result<User> {
        let auth = ctx.data::<AuthenticatedUser>()?;
        let service = ctx.data::<UserService>()?;

        service.update(auth.user_id, input).await
    }

    /// Delete a user (admin only).
    #[graphql(guard = "RoleGuard::new(Role::Admin)")]
    async fn delete_user(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> Result<bool> {
        let service = ctx.data::<UserService>()?;
        service.delete(id.parse()?).await?;
        Ok(true)
    }

    /// Authenticate and get tokens.
    async fn login(
        &self,
        ctx: &Context<'_>,
        email: String,
        password: String,
    ) -> Result<AuthPayload> {
        let auth_service = ctx.data::<AuthService>()?;
        auth_service.authenticate(&email, &password).await
    }
}
```

## Subscription Implementation

```rust
pub struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
    /// Subscribe to new messages in a chat room.
    async fn messages(
        &self,
        ctx: &Context<'_>,
        room_id: ID,
    ) -> impl Stream<Item = Message> {
        let broadcaster = ctx.data_unchecked::<MessageBroadcaster>();
        let room_id: i64 = room_id.parse().unwrap();

        broadcaster.subscribe(room_id)
    }

    /// Subscribe to user presence updates.
    async fn user_presence(
        &self,
        ctx: &Context<'_>,
    ) -> impl Stream<Item = PresenceUpdate> {
        let presence = ctx.data_unchecked::<PresenceService>();
        presence.subscribe()
    }

    /// Subscribe to notifications for the current user.
    #[graphql(guard = "AuthGuard")]
    async fn notifications(
        &self,
        ctx: &Context<'_>,
    ) -> Result<impl Stream<Item = Notification>> {
        let auth = ctx.data::<AuthenticatedUser>()?;
        let notifications = ctx.data::<NotificationService>()?;

        Ok(notifications.subscribe(auth.user_id))
    }
}
```

## Data Loaders (N+1 Prevention)

```rust
use async_graphql::dataloader::*;

pub struct PostLoader {
    pool: PgPool,
}

impl Loader<i64> for PostLoader {
    type Value = Vec<Post>;
    type Error = Error;

    async fn load(&self, keys: &[i64]) -> Result<HashMap<i64, Self::Value>, Self::Error> {
        let posts = sqlx::query_as!(
            Post,
            r#"SELECT * FROM posts WHERE user_id = ANY($1)"#,
            keys
        )
        .fetch_all(&self.pool)
        .await?;

        // Group posts by user_id
        let mut map: HashMap<i64, Vec<Post>> = HashMap::new();
        for post in posts {
            map.entry(post.user_id).or_default().push(post);
        }

        Ok(map)
    }
}

// Register in schema
let schema = Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
    .data(DataLoader::new(PostLoader { pool: pool.clone() }, tokio::spawn))
    .finish();
```

## Guards for Authorization

```rust
use async_graphql::*;

/// Guard that requires authentication.
pub struct AuthGuard;

#[async_trait::async_trait]
impl Guard for AuthGuard {
    async fn check(&self, ctx: &Context<'_>) -> Result<()> {
        if ctx.data_opt::<AuthenticatedUser>().is_some() {
            Ok(())
        } else {
            Err("Unauthorized".into())
        }
    }
}

/// Guard that requires a specific role.
pub struct RoleGuard {
    required_role: Role,
}

impl RoleGuard {
    pub fn new(role: Role) -> Self {
        Self { required_role: role }
    }
}

#[async_trait::async_trait]
impl Guard for RoleGuard {
    async fn check(&self, ctx: &Context<'_>) -> Result<()> {
        let auth = ctx.data_opt::<AuthenticatedUser>()
            .ok_or("Unauthorized")?;

        if auth.role >= self.required_role {
            Ok(())
        } else {
            Err("Forbidden".into())
        }
    }
}
```

## Schema Integration with Armature

```rust
use armature::prelude::*;
use armature_graphql::*;

#[module(
    providers: [
        GraphQLSchemaProvider,
        UserService,
        PostService,
        AuthService,
    ],
    controllers: [GraphQLController],
)]
pub struct GraphQLModule;

#[injectable]
pub struct GraphQLSchemaProvider {
    user_service: UserService,
    post_service: PostService,
    auth_service: AuthService,
}

impl GraphQLSchemaProvider {
    pub fn schema(&self) -> Schema<QueryRoot, MutationRoot, SubscriptionRoot> {
        Schema::build(QueryRoot, MutationRoot, SubscriptionRoot)
            .data(self.user_service.clone())
            .data(self.post_service.clone())
            .data(self.auth_service.clone())
            .limit_depth(10)
            .limit_complexity(1000)
            .finish()
    }
}

#[controller("/graphql")]
pub struct GraphQLController {
    schema_provider: GraphQLSchemaProvider,
}

impl GraphQLController {
    #[post("/")]
    async fn execute(
        &self,
        req: HttpRequest,
        body: Json<GraphQLRequest>,
    ) -> Result<Json<GraphQLResponse>, Error> {
        // Extract auth from request
        let auth = extract_auth(&req);

        let mut request = body.0.into_inner();
        if let Some(auth) = auth {
            request = request.data(auth);
        }

        let schema = self.schema_provider.schema();
        let response = schema.execute(request).await;

        Ok(Json(response.into()))
    }

    #[get("/")]
    async fn playground(&self) -> HttpResponse {
        HttpResponse::ok()
            .with_header("Content-Type", "text/html")
            .with_body(playground_source(
                GraphQLPlaygroundConfig::new("/graphql")
                    .subscription_endpoint("/graphql/ws")
            ))
    }

    #[get("/ws")]
    async fn subscriptions(
        &self,
        ws: WebSocketUpgrade,
    ) -> WebSocketResponse {
        let schema = self.schema_provider.schema();
        ws.on_upgrade(move |socket| async move {
            GraphQLSubscription::new(schema)
                .serve(socket)
                .await
        })
    }
}
```

## Error Handling

```rust
use async_graphql::*;

/// Custom error type for GraphQL.
#[derive(Debug, thiserror::Error)]
pub enum GraphQLError {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Forbidden")]
    Forbidden,

    #[error("Internal error")]
    Internal(#[from] anyhow::Error),
}

impl ErrorExtensions for GraphQLError {
    fn extend(&self) -> Error {
        let (code, message) = match self {
            Self::NotFound(msg) => ("NOT_FOUND", msg.as_str()),
            Self::Validation(msg) => ("VALIDATION_ERROR", msg.as_str()),
            Self::Unauthorized => ("UNAUTHORIZED", "Authentication required"),
            Self::Forbidden => ("FORBIDDEN", "Insufficient permissions"),
            Self::Internal(_) => ("INTERNAL_ERROR", "An internal error occurred"),
        };

        Error::new(message).extend_with(|_, e| {
            e.set("code", code);
        })
    }
}

// Usage
async fn get_user(&self, ctx: &Context<'_>, id: ID) -> Result<User> {
    let service = ctx.data::<UserService>()?;
    service.find_by_id(id.parse()?)
        .await?
        .ok_or_else(|| GraphQLError::NotFound(format!("User {} not found", id)).extend())
}
```

## Best Practices

1. **Use DataLoaders** to prevent N+1 queries
2. **Limit query depth** and complexity
3. **Use guards** for authorization
4. **Validate inputs** with built-in validators
5. **Document everything** with `///` comments
6. **Use connections** for pagination
7. **Handle errors** with proper codes
8. **Use subscriptions** for real-time updates
9. **Separate concerns** - types, resolvers, services
10. **Test queries** thoroughly

## Summary

```graphql
# Example query
query GetUser($id: ID!) {
  user(id: $id) {
    id
    name
    email
    posts {
      id
      title
    }
  }
}

# Example mutation
mutation CreateUser($input: CreateUserInput!) {
  createUser(input: $input) {
    id
    name
    email
  }
}

# Example subscription
subscription OnMessage($roomId: ID!) {
  messages(roomId: $roomId) {
    id
    content
    sender {
      name
    }
  }
}
```


## Cursor rule: `.cursor/rules/middleware-development.mdc`

_Guidelines for creating middleware in Armature_

Applies to: `**/middleware/**/*.rs, **/interceptors/**/*.rs`

# Middleware Development

Standards for creating middleware and interceptors in Armature.

## Middleware Structure

```rust
use armature_core::{Middleware, Request, Response, Next};

pub struct LoggingMiddleware {
    logger: Arc<Logger>,
}

impl Middleware for LoggingMiddleware {
    async fn handle(&self, req: Request, next: Next) -> Result<Response, Error> {
        let start = Instant::now();
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        // Before handler
        tracing::info!(%method, %path, "Request started");

        // Call next middleware/handler
        let response = next.run(req).await?;

        // After handler
        let duration = start.elapsed();
        tracing::info!(
            %method,
            %path,
            status = %response.status(),
            duration_ms = %duration.as_millis(),
            "Request completed"
        );

        Ok(response)
    }
}
```

## Common Middleware Types

### Request ID

```rust
pub struct RequestIdMiddleware;

impl Middleware for RequestIdMiddleware {
    async fn handle(&self, mut req: Request, next: Next) -> Result<Response, Error> {
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        req.extensions_mut().insert(RequestId(request_id.clone()));

        let mut response = next.run(req).await?;
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_str(&request_id)?,
        );

        Ok(response)
    }
}
```

### Rate Limiting

```rust
pub struct RateLimitMiddleware {
    limiter: Arc<RateLimiter>,
}

impl Middleware for RateLimitMiddleware {
    async fn handle(&self, req: Request, next: Next) -> Result<Response, Error> {
        let key = extract_client_key(&req);

        if !self.limiter.check(&key).await {
            return Err(Error::TooManyRequests);
        }

        next.run(req).await
    }
}
```

### CORS

```rust
pub struct CorsMiddleware {
    config: CorsConfig,
}

impl Middleware for CorsMiddleware {
    async fn handle(&self, req: Request, next: Next) -> Result<Response, Error> {
        // Handle preflight
        if req.method() == Method::OPTIONS {
            return Ok(self.preflight_response(&req));
        }

        let mut response = next.run(req).await?;

        // Add CORS headers
        let headers = response.headers_mut();
        headers.insert("access-control-allow-origin", self.config.origin.parse()?);
        headers.insert("access-control-allow-methods", self.config.methods.parse()?);
        headers.insert("access-control-allow-headers", self.config.headers.parse()?);

        Ok(response)
    }
}
```

## Interceptors

Interceptors wrap handlers with before/after logic:

```rust
#[interceptor]
pub struct TimingInterceptor;

impl Interceptor for TimingInterceptor {
    async fn before(&self, ctx: &mut RequestContext) -> Result<(), Error> {
        ctx.extensions_mut().insert(StartTime(Instant::now()));
        Ok(())
    }

    async fn after(&self, ctx: &RequestContext, response: &mut Response) -> Result<(), Error> {
        if let Some(StartTime(start)) = ctx.extensions().get::<StartTime>() {
            let duration = start.elapsed();
            response.headers_mut().insert(
                "x-response-time",
                HeaderValue::from_str(&format!("{}ms", duration.as_millis()))?,
            );
        }
        Ok(())
    }
}
```

## Error Handling Middleware

```rust
pub struct ErrorHandlerMiddleware;

impl Middleware for ErrorHandlerMiddleware {
    async fn handle(&self, req: Request, next: Next) -> Result<Response, Error> {
        match next.run(req).await {
            Ok(response) => Ok(response),
            Err(error) => {
                // Log the error
                tracing::error!(?error, "Request failed");

                // Convert to response
                Ok(error.into_response())
            }
        }
    }
}
```

## Middleware Registration

```rust
let app = App::new()
    .middleware(RequestIdMiddleware)
    .middleware(LoggingMiddleware::new(logger))
    .middleware(CorsMiddleware::new(cors_config))
    .middleware(RateLimitMiddleware::new(limiter))
    .middleware(ErrorHandlerMiddleware);
```

## Order Matters

Middleware executes in registration order (outside-in):

```
Request  → RequestId → Logging → CORS → Handler
Response ← RequestId ← Logging ← CORS ← Handler
```

## Testing Middleware

```rust
#[tokio::test]
async fn test_request_id_middleware() {
    let middleware = RequestIdMiddleware;

    let req = Request::builder()
        .uri("/test")
        .body(Body::empty())?;

    let next = Next::new(|req| async {
        Ok(Response::new(Body::empty()))
    });

    let response = middleware.handle(req, next).await?;

    assert!(response.headers().contains_key("x-request-id"));
}
```


## Cursor rule: `.cursor/rules/module-development.mdc`

# Armature Module Development

Guidelines for creating new modules/crates in the Armature framework.

## Module Structure

Every new armature module should follow this structure:

```
armature-<name>/
├── Cargo.toml
├── src/
│   ├── lib.rs           # Public API exports
│   ├── config.rs        # Configuration types (if applicable)
│   ├── error.rs         # Module-specific error types
│   ├── traits.rs        # Core traits for the module
│   └── ...              # Implementation files
└── tests/
    └── integration.rs   # Integration tests
```

## Cargo.toml Template

```toml
[package]
name = "armature-<name>"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
description = "Brief description of the module"
keywords = ["armature", "<relevant>", "<keywords>"]
categories = ["web-programming"]

[dependencies]
# Core dependencies - use workspace versions when available
tokio = { version = "1.35", features = ["full"] }
async-trait = "0.1"
thiserror = "2.0"
serde = { version = "1.0", features = ["derive"] }

# Optional: armature-core for DI integration
armature-core = { path = "../armature-core", version = "0.1.0", optional = true }

[features]
default = []
# Feature for DI integration
di = ["armature-core"]

[dev-dependencies]
tokio-test = "0.4"
```

## Error Handling Pattern

```rust
// src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModuleError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Operation failed: {0}")]
    Operation(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, ModuleError>;
```

## Configuration Pattern

```rust
// src/config.rs
use serde::{Deserialize, Serialize};

/// Configuration for the module.
///
/// # Examples
///
/// ```rust
/// use armature_<name>::Config;
///
/// let config = Config::builder()
///     .option1("value")
///     .option2(42)
///     .build();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub option1: String,
    pub option2: i32,
    #[serde(default)]
    pub optional_field: Option<String>,
}

impl Config {
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }

    pub fn from_env() -> ConfigBuilder {
        ConfigBuilder::from_env()
    }
}

#[derive(Debug, Default)]
pub struct ConfigBuilder {
    option1: Option<String>,
    option2: Option<i32>,
    optional_field: Option<String>,
}

impl ConfigBuilder {
    pub fn from_env() -> Self {
        Self {
            option1: std::env::var("MODULE_OPTION1").ok(),
            option2: std::env::var("MODULE_OPTION2").ok().and_then(|v| v.parse().ok()),
            optional_field: std::env::var("MODULE_OPTIONAL").ok(),
        }
    }

    pub fn option1(mut self, value: impl Into<String>) -> Self {
        self.option1 = Some(value.into());
        self
    }

    pub fn option2(mut self, value: i32) -> Self {
        self.option2 = Some(value);
        self
    }

    pub fn optional_field(mut self, value: impl Into<String>) -> Self {
        self.optional_field = Some(value.into());
        self
    }

    pub fn build(self) -> Config {
        Config {
            option1: self.option1.unwrap_or_default(),
            option2: self.option2.unwrap_or(0),
            optional_field: self.optional_field,
        }
    }
}
```

## Service Pattern with DI Integration

```rust
// src/service.rs
use crate::{Config, Result};

/// Main service for the module.
///
/// Supports automatic dependency injection when the `di` feature is enabled.
#[derive(Clone)]
pub struct ModuleService {
    config: Config,
    // Internal state
}

impl ModuleService {
    /// Create a new service with the given configuration.
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Create with default configuration.
    pub fn default() -> Self {
        Self::new(Config::builder().build())
    }

    /// Primary operation of this module.
    pub async fn do_something(&self, input: &str) -> Result<String> {
        // Implementation
        Ok(format!("Processed: {}", input))
    }
}

// DI integration (when feature enabled)
#[cfg(feature = "di")]
mod di {
    use super::*;
    use armature_core::prelude::*;

    impl Provider for ModuleService {
        fn create(_container: &Container) -> std::result::Result<Self, armature_core::Error> {
            Ok(Self::default())
        }
    }
}
```

## Trait Definition Pattern

```rust
// src/traits.rs
use async_trait::async_trait;
use crate::Result;

/// Core trait for module implementations.
///
/// Implement this trait to create custom backends.
#[async_trait]
pub trait Backend: Send + Sync {
    /// Initialize the backend.
    async fn init(&mut self) -> Result<()>;

    /// Perform the main operation.
    async fn execute(&self, input: &str) -> Result<String>;

    /// Clean up resources.
    async fn shutdown(&mut self) -> Result<()>;
}
```

## lib.rs Structure

```rust
//! Armature <Name> Module
//!
//! This module provides <brief description>.
//!
//! # Features
//!
//! - Feature 1
//! - Feature 2
//!
//! # Examples
//!
//! ```rust,no_run
//! use armature_<name>::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let service = ModuleService::new(
//!         Config::builder()
//!             .option1("value")
//!             .build()
//!     );
//!
//!     let result = service.do_something("input").await?;
//!     println!("{}", result);
//!     Ok(())
//! }
//! ```

mod config;
mod error;
mod service;
mod traits;

// Re-export public API
pub use config::{Config, ConfigBuilder};
pub use error::{ModuleError, Result};
pub use service::ModuleService;
pub use traits::Backend;

/// Prelude for convenient imports.
pub mod prelude {
    pub use crate::{Config, ModuleError, ModuleService, Result};
}
```

## Adding to Workspace

After creating the module, add it to the workspace `Cargo.toml`:

```toml
[workspace]
members = [
    # ... existing members
    "armature-<name>",
]
```

If the module should be optional in the main framework:

```toml
# In root Cargo.toml
[dependencies]
armature-<name> = { path = "armature-<name>", version = "0.1.0", optional = true }

[features]
<name> = ["armature-<name>"]
full = [
    # ... existing features
    "<name>",
]
```

## Documentation Requirements

Every new module MUST have:

1. **Inline documentation** - `///` comments on all public items
2. **Module documentation** - `//!` at top of lib.rs
3. **Examples** - In doc comments and in `examples/` directory
4. **Feature guide** - In `docs/<name>-guide.md`

## Testing Requirements

1. **Unit tests** - In source files with `#[cfg(test)]`
2. **Integration tests** - In `tests/` directory
3. **Doc tests** - In documentation examples
4. **Coverage** - Aim for 85%+ coverage

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_operation() {
        let service = ModuleService::default();
        let result = service.do_something("test").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Processed: test");
    }

    #[test]
    fn test_config_builder() {
        let config = Config::builder()
            .option1("value")
            .option2(42)
            .build();

        assert_eq!(config.option1, "value");
        assert_eq!(config.option2, 42);
    }
}
```

## Checklist for New Modules

- [ ] Created `armature-<name>/` directory
- [ ] Created `Cargo.toml` with workspace inheritance
- [ ] Created `src/lib.rs` with module documentation
- [ ] Created `src/error.rs` with module-specific errors
- [ ] Created `src/config.rs` with builder pattern
- [ ] Created core service/trait implementations
- [ ] Added to workspace `Cargo.toml`
- [ ] Added optional feature to root `Cargo.toml`
- [ ] Added unit tests
- [ ] Added integration tests
- [ ] Created `docs/<name>-guide.md`
- [ ] Added example in `examples/`
- [ ] All clippy warnings resolved
- [ ] All tests passing


## Cursor rule: `.cursor/rules/no-direct-commits.mdc`

# No Direct Commits to Main and Develop

**CRITICAL RULE:** Never commit directly to `main` or `develop` branches.

## Protected Branches

### `main` branch
- ❌ **NO direct commits**
- ❌ **NO force pushes**
- ✅ Only merge via Pull Requests from `release/*` or `hotfix/*`

### `develop` branch
- ❌ **NO direct commits**
- ❌ **NO force pushes**
- ✅ Only merge via Pull Requests from `feature/*`, `release/*`, or `hotfix/*`

## Why This Rule Exists

1. **Code Review:** All changes must be reviewed before merging
2. **CI/CD:** Automated tests must pass before integration
3. **Quality Control:** Prevents untested code from reaching protected branches
4. **Audit Trail:** Maintains clear history of what changed and why
5. **Team Collaboration:** Ensures visibility of all changes
6. **Rollback Safety:** Makes it easier to revert problematic changes

## Correct Workflow

### For New Features

```bash
# ❌ WRONG - Never do this!
git checkout develop
git add .
git commit -m "Add new feature"
git push origin develop  # This will be rejected!

# ✅ CORRECT
git checkout develop
git pull origin develop
git checkout -b feature/123-new-feature
git add .
git commit -m "feat: add new feature"
git push origin feature/123-new-feature
# Then create Pull Request on GitHub/GitLab
```

### For Bug Fixes

```bash
# ❌ WRONG
git checkout develop
git commit -am "fix bug"
git push origin develop  # This will be rejected!

# ✅ CORRECT
git checkout develop
git pull origin develop
git checkout -b feature/456-fix-bug
git commit -am "fix: resolve issue with authentication"
git push origin feature/456-fix-bug
# Then create Pull Request
```

### For Hotfixes

```bash
# ❌ WRONG
git checkout main
git commit -am "urgent fix"
git push origin main  # This will be rejected!

# ✅ CORRECT
git checkout main
git pull origin main
git checkout -b hotfix/1.0.1-critical-fix
git commit -am "fix: resolve critical security issue"
git push origin hotfix/1.0.1-critical-fix
# Then create Pull Request to main AND develop
```

## What If I Accidentally Commit?

### Before Pushing

If you committed to `main` or `develop` locally but haven't pushed yet:

```bash
# Move the commit to a new branch
git branch feature/my-changes
git reset --hard origin/develop  # or origin/main
git checkout feature/my-changes
git push origin feature/my-changes
# Now create Pull Request
```

### After Pushing (If Allowed)

If you somehow managed to push directly:

```bash
# Immediately notify the team
# Revert the commit
git checkout develop  # or main
git revert HEAD
git push origin develop  # or main

# Then create proper feature branch with the fix
git checkout -b feature/proper-implementation
# Re-apply your changes properly
git push origin feature/proper-implementation
# Create Pull Request
```

## Emergency Exceptions

In **extremely rare** emergency situations (production down, data loss, security breach), a direct commit *might* be necessary:

### Emergency Procedure

1. **Get approval** from team lead/CTO
2. **Document reason** in commit message
3. **Notify team** immediately in Slack/Discord
4. **Create follow-up PR** with proper testing
5. **Post-mortem** document after resolution

```bash
# Only in extreme emergency with approval
git checkout main
git commit -am "EMERGENCY: fix critical production outage

Reason: Database connection pool exhausted causing 100% error rate
Approved by: [Name]
Impact: 10,000+ users affected
Ticket: #CRITICAL-123"
git push origin main

# Immediately after, create proper PR for review
git checkout -b hotfix/emergency-followup
# Add tests, documentation, etc.
git push origin hotfix/emergency-followup
```

## Pull Request Requirements

All merges to `main` and `develop` must go through Pull Requests with:

### Required Checks

- ✅ At least one approval from code owner
- ✅ All CI tests passing (`cargo test --all-features`)
- ✅ No clippy warnings (`cargo clippy -- -D warnings`)
- ✅ Code is formatted (`cargo fmt -- --check`)
- ✅ No merge conflicts
- ✅ Branch is up to date with target
- ✅ All conversations resolved

### PR Must Include

- Clear description of changes
- Link to related issue/ticket
- Test coverage for new code
- Documentation updates (if needed)
- CHANGELOG update (for releases)

## Branch Protection Setup

### GitHub Settings

```yaml
# .github/branch-protection.yml (conceptual)
branches:
  main:
    protection:
      required_pull_request_reviews:
        required_approving_review_count: 1
        dismiss_stale_reviews: true
      required_status_checks:
        strict: true
        contexts:
          - "test"
          - "lint"
          - "format-check"
      enforce_admins: true
      restrictions: null

  develop:
    protection:
      required_pull_request_reviews:
        required_approving_review_count: 1
      required_status_checks:
        strict: true
        contexts:
          - "test"
          - "lint"
      enforce_admins: true
```

## Common Mistakes to Avoid

### ❌ Mistake 1: "Just a quick fix"

```bash
# NO! Even small changes need PR
git checkout develop
git commit -am "fix typo"  # Still wrong!
```

### ❌ Mistake 2: "Nobody will notice"

```bash
# Everyone will notice, and CI should block it
git push origin main  # Protected branch!
```

### ❌ Mistake 3: "I'm the only developer"

```bash
# Still wrong - maintains good habits and audit trail
git commit --allow-empty -m "update"
git push origin develop  # Bad practice!
```

### ❌ Mistake 4: Force pushing to fix

```bash
# NEVER force push to protected branches
git push --force origin main  # Extremely bad!
```

## Enforcement

### Git Hooks

Add a pre-push hook to prevent accidents:

```bash
#!/bin/bash
# .git/hooks/pre-push

protected_branches='main develop'
current_branch=$(git symbolic-ref HEAD | sed -e 's,.*/\(.*\),\1,')

for branch in $protected_branches; do
    if [ $branch = $current_branch ]; then
        echo "🚫 Direct push to $current_branch is not allowed!"
        echo "   Create a feature branch and use a Pull Request."
        echo "   Run: git checkout -b feature/your-feature-name"
        exit 1
    fi
done

exit 0
```

### Make it executable

```bash
chmod +x .git/hooks/pre-push
```

## Team Communication

If you see someone attempting to push directly to `main` or `develop`:

### Gentle Reminder

> Hey! I noticed you're trying to commit directly to [main/develop].
> Let's create a feature branch instead so we can get it reviewed:
>
> ```
> git checkout -b feature/your-change
> git push origin feature/your-change
> ```
>
> Then create a PR and I'll review it! 👍

## Summary

### The Golden Rules

1. ✅ **Always** create a feature/hotfix branch
2. ✅ **Always** push to your branch
3. ✅ **Always** create a Pull Request
4. ✅ **Always** wait for review and approval
5. ✅ **Always** ensure CI passes

### Never Do This

1. ❌ **Never** `git checkout main` and commit
2. ❌ **Never** `git checkout develop` and commit
3. ❌ **Never** force push to protected branches
4. ❌ **Never** bypass CI checks
5. ❌ **Never** merge without approval

### Quick Reference

```bash
# The right way every time:
git checkout develop
git pull origin develop
git checkout -b feature/my-feature
# Make changes...
git add .
git commit -m "feat: add feature"
git push origin feature/my-feature
# Create PR on GitHub
```

---

**Remember:** If it feels like you're taking a shortcut, you probably are.
Use branches and PRs - it's worth the extra 30 seconds! 🚀


## Cursor rule: `.cursor/rules/no-unsolicited-documentation.mdc`

# No Unsolicited Documentation or Summaries

## Rule

**DO NOT** generate summaries, markdown documentation, or other documentation files unless the user explicitly requests them.

## What NOT to Do

❌ **Don't create unsolicited summaries:**
- Task summaries
- Implementation summaries
- Session summaries
- Progress reports
- Markdown recap documents

❌ **Don't create unsolicited documentation:**
- README files (unless explicitly requested)
- CHANGELOG updates (unless explicitly requested)
- Implementation guides
- Status reports
- Progress documentation

❌ **Don't add verbose explanations:**
- Long explanations of what was done
- Detailed breakdowns of changes
- Extensive commit message formatting in responses

## What TO Do

✅ **Only provide:**
- Direct answers to questions
- Implementation of requested features
- Error fixes and corrections
- Brief confirmations of completed work

✅ **Create documentation when:**
- User explicitly asks: "create a README", "document this", "write a guide"
- Project documentation standards require it (see `documentation.mdc`)
- Feature documentation is required by workspace rules

✅ **Keep responses concise:**
- Focus on the task at hand
- Provide essential information only
- Let the work speak for itself

## Examples

### ❌ Bad (Unsolicited Summary)

```
I've completed the task. Here's what I did:

## Summary
- Created 5 new files
- Modified 3 existing files
- Added comprehensive documentation

## Files Changed
1. file1.rs - Added feature X
2. file2.rs - Fixed bug Y
...
(extensive breakdown)
```

### ✅ Good (Concise Confirmation)

```
Task complete. Created the authentication module with JWT support.
```

### ❌ Bad (Creating Unsolicited Docs)

```
Let me also create a SUMMARY.md to document what we've done...
```

### ✅ Good (Only When Asked)

```
User: "Create a summary of the authentication features"
Assistant: (creates SUMMARY.md)
```

## Exception: Required Documentation

The **only exception** is when documentation is required by workspace rules:

1. **Feature documentation** - As per `documentation.mdc`, every feature MUST have documentation in `docs/`
2. **API documentation** - Inline Rust doc comments (`///`) are required
3. **Examples** - Working examples are required for new features

These are NOT optional and should be created automatically with new features.

## Summary

**Key Principle:** Only generate what is explicitly requested or required by project standards. Don't create summaries, recaps, or extra documentation files unless asked.


## Cursor rule: `.cursor/rules/performance-benchmarking.mdc`

# Performance & Benchmarking

Guidelines for performance optimization and benchmarking in the Armature framework.

## Benchmark Infrastructure

### Running Benchmarks

```bash
# Run every suite
./scripts/run-benchmarks.sh --all

# Run specific benchmark (always `-p`-scoped to the owning crate)
cargo bench -p armature-core --bench core

# Run with native CPU optimizations
cargo bench -p armature-core --profile release-native

# Run with flamegraph profiling
cargo bench -p armature-core --profile profiling
```

### Benchmark Location

Each benchmark lives in the crate it measures:

```
armature-core/benches/        # core, router, arena, body, json, micro,
                              # pipeline, resilience, simd_parser,
                              # internal_overhead
armature-h1/benches/          # parse, write, e2e
armature-jwt/benches/         # jwt (signing/verification)
armature-auth/benches/        # auth (hashing, guards, OAuth2)
armature-validation/benches/  # validation
armature-cache/benches/       # cache
armature-cron/benches/        # cron
armature-queue/benches/       # queue
armature-ratelimit/benches/   # ratelimit
armature-storage/benches/     # storage
armature-http-client/benches/ # http_client
```

The root `benches/` keeps only what is not crate-specific: the framework
comparison servers, the TechEmpower harness, the `http-benchmark` load runner,
and the `database_benchmarks`/`memory_benchmarks` pattern benchmarks.

Crates with benchmarks set `autobenches = false`, so a new file needs an
explicit `[[bench]]` entry in that crate's `Cargo.toml`.

## Writing Benchmarks with Criterion

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};

fn benchmark_router(c: &mut Criterion) {
    let router = Router::new()
        .route("/users", get(users_handler))
        .route("/users/:id", get(user_handler));

    c.bench_function("router_static_route", |b| {
        b.iter(|| {
            black_box(router.match_route("/users", Method::GET))
        })
    });

    c.bench_function("router_dynamic_route", |b| {
        b.iter(|| {
            black_box(router.match_route("/users/123", Method::GET))
        })
    });
}

// Parameterized benchmarks
fn benchmark_json_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_parsing");

    for size in [100, 1000, 10000].iter() {
        let json = generate_json(*size);

        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &json,
            |b, json| {
                b.iter(|| {
                    black_box(serde_json::from_str::<Value>(json).unwrap())
                })
            },
        );
    }

    group.finish();
}

criterion_group!(benches, benchmark_router, benchmark_json_parsing);
criterion_main!(benches);
```

## Async Benchmarks

```rust
use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Runtime;

fn benchmark_async_handler(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    c.bench_function("async_handler", |b| {
        b.to_async(&rt).iter(|| async {
            let response = handle_request().await;
            black_box(response)
        })
    });
}
```

## Profiling

### CPU Profiling with Flamegraph

```bash
# Install flamegraph
cargo install flamegraph

# Generate flamegraph
cargo flamegraph -p armature-core --bench core -- --bench

# Or for a running server
cargo flamegraph --example profiling_server
```

### Memory Profiling

The project has comprehensive memory profiling tools:

```bash
# Use the memory profiling script
./scripts/memory-profile.sh dhat 30      # DHAT (recommended for Rust)
./scripts/memory-profile.sh valgrind 30  # Valgrind leak detection
./scripts/memory-profile.sh massif 30    # Massif heap profiler
./scripts/memory-profile.sh heaptrack 30 # Heaptrack detailed analysis

# Build with DHAT support
cargo build --example memory_profile_server --release --features memory-profiling

# Run memory benchmarks
cargo bench -p armature-framework --bench memory_benchmarks
```

**DHAT Setup:**

```rust
#[cfg(feature = "memory-profiling")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

fn main() {
    #[cfg(feature = "memory-profiling")]
    let _profiler = dhat::Profiler::new_heap();
    // Run workload - report generated on exit
}
```

View DHAT reports at: https://nnethercote.github.io/dh_view/dh_view.html

See `docs/memory-profiling-guide.md` for complete documentation.

### Using perf

```bash
# Record performance data
perf record -g cargo bench -p armature-core --bench core

# Generate report
perf report

# Generate flamegraph from perf data
perf script | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg
```

## Build Profiles

The project has optimized build profiles in `Cargo.toml`:

| Profile | Use Case | LTO | Optimizations |
|---------|----------|-----|---------------|
| `release` | Standard release | thin | O3 |
| `release-fat` | Maximum optimization | fat | O3 + panic=abort |
| `release-native` | Benchmarks | thin | O3 + target-cpu=native |
| `profiling` | Profiling | thin | O3 + debug symbols |
| `pgo-generate` | PGO data collection | thin | O3 |
| `pgo-use` | PGO-optimized build | fat | O3 |

### Profile-Guided Optimization (PGO)

```bash
# Step 1: Build with profiling instrumentation
RUSTFLAGS="-Cprofile-generate=/tmp/pgo" cargo build --profile pgo-generate

# Step 2: Run representative workload
./target/pgo-generate/armature-benchmark

# Step 3: Merge profile data
llvm-profdata merge -o merged.profdata /tmp/pgo/*.profraw

# Step 4: Build with PGO
RUSTFLAGS="-Cprofile-use=$(pwd)/merged.profdata" cargo build --profile pgo-use
```

## Performance Patterns

### Zero-Cost Abstractions

```rust
// ✅ Good: Zero-cost abstraction with generics
pub fn process<T: AsRef<[u8]>>(data: T) -> Result<(), Error> {
    let bytes = data.as_ref();
    // Process bytes
    Ok(())
}

// ❌ Bad: Dynamic dispatch when not needed
pub fn process(data: &dyn AsRef<[u8]>) -> Result<(), Error> {
    // Unnecessary vtable lookup
}
```

### Minimize Allocations

```rust
// ✅ Good: Reuse buffers
pub struct Router {
    buffer: Vec<u8>,  // Reused across requests
}

impl Router {
    pub fn handle(&mut self, request: &[u8]) -> Response {
        self.buffer.clear();
        self.buffer.extend_from_slice(request);
        // Process using self.buffer
    }
}

// ❌ Bad: Allocate per request
pub fn handle(request: &[u8]) -> Response {
    let buffer = request.to_vec();  // New allocation every time
    // Process
}
```

### Use SmallVec for Small Collections

```rust
use smallvec::SmallVec;

// ✅ Good: Stack allocation for typical case
type Headers = SmallVec<[(String, String); 16]>;

// ❌ Bad: Always heap allocate
type Headers = Vec<(String, String)>;
```

### Inline Hot Paths

```rust
// ✅ Good: Inline small, hot functions
#[inline]
pub fn parse_method(bytes: &[u8]) -> Option<Method> {
    match bytes {
        b"GET" => Some(Method::Get),
        b"POST" => Some(Method::Post),
        _ => None,
    }
}

// For very hot paths
#[inline(always)]
pub fn is_whitespace(b: u8) -> bool {
    b == b' ' || b == b'\t'
}
```

### SIMD Optimization

The framework includes SIMD-optimized parsers:

```rust
// Enable SIMD JSON parsing
[features]
simd-json = ["armature-core/simd-json"]

// Use SIMD parser for HTTP parsing
use armature_core::simd_parser::*;

let (method, path, version) = parse_request_line_simd(bytes)?;
```

## Benchmark Targets

### Framework Comparison Baselines

| Operation | Target | Current |
|-----------|--------|---------|
| Hello World RPS | 500k+ | - |
| JSON Serialization | 200k+ RPS | - |
| Route Matching | 1M+ ops/sec | - |
| JWT Validation | 50k+ ops/sec | - |

### Compare Against

- Actix-web
- Axum
- Warp
- Rocket

```bash
# Run comparison benchmarks
cargo run --release --bin http-benchmark -- --all
```

## Memory Optimization

### Track Allocations

```rust
#[cfg(feature = "alloc-tracking")]
use tracking_allocator::TrackingAllocator;

#[cfg(feature = "alloc-tracking")]
#[global_allocator]
static ALLOC: TrackingAllocator = TrackingAllocator;

// In tests
#[test]
fn test_no_allocations() {
    let _guard = ALLOC.track();

    // This should not allocate
    let result = hot_path_function();

    assert_eq!(ALLOC.allocations(), 0);
}
```

### Arena Allocation

```rust
use armature_core::arena::Arena;

// ✅ Good: Arena for request-scoped allocations
pub async fn handle_request(arena: &Arena) -> Response {
    let headers = arena.alloc_slice(&parsed_headers);
    let body = arena.alloc_str(&body_content);
    // All deallocated at once when request completes
}
```

## Continuous Benchmarking

### CI Integration

```yaml
# .github/workflows/bench.yml
name: Benchmarks

on:
  push:
    branches: [main]
  pull_request:

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Run benchmarks
        run: cargo bench -p armature-core --bench core -- --save-baseline pr

      - name: Compare with main
        run: |
          git checkout main
          cargo bench -p armature-core --bench core -- --baseline main
          git checkout -
          critcmp main pr
```

## Summary

1. Use **Criterion** for benchmarking with proper methodology
2. Profile with **flamegraph** and **perf** before optimizing
3. Use **release-native** profile for accurate benchmark numbers
4. Consider **PGO** for production builds
5. Minimize allocations in hot paths
6. Use **SmallVec** for small, bounded collections
7. Leverage **SIMD** for parsing operations
8. Track and compare benchmarks in CI


## Cursor rule: `.cursor/rules/performance-optimization.mdc`

_Performance optimization and profiling guidelines for Armature_

Applies to: `benches/**/*.rs, armature-*/benches/**/*.rs, examples/*profile*.rs, scripts/*profile*.sh`

# Performance Optimization

Guidelines for optimizing performance and profiling the Armature framework.

## Benchmarking with Criterion

```rust
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

fn bench_request_parsing(c: &mut Criterion) {
    let data = setup_test_data();

    c.bench_function("parse_request", |b| {
        b.iter(|| {
            black_box(parse_request(black_box(&data)))
        })
    });
}

criterion_group!(benches, bench_request_parsing);
criterion_main!(benches);
```

## Memory Optimization

### Arena Allocation

Use arena allocators for request-scoped data:

```rust
use armature_core::arena::{with_arena, reset_arena};

async fn handle_request(req: Request) -> Response {
    with_arena(|arena| {
        // Allocations here are freed together
        let data = arena.alloc_str(&req.body);
        process(data)
    });

    reset_arena(); // Free all at once
}
```

### Object Pools

Reuse frequently allocated objects:

```rust
use crossbeam::queue::ArrayQueue;

pub struct Pool<T> {
    objects: ArrayQueue<T>,
    factory: fn() -> T,
}

impl<T> Pool<T> {
    pub fn get(&self) -> PoolGuard<T> {
        let obj = self.objects.pop().unwrap_or_else(|| (self.factory)());
        PoolGuard { pool: self, obj: Some(obj) }
    }
}
```

### Bounded Collections

Always bound caches to prevent memory leaks:

```rust
use lru::LruCache;
use std::num::NonZeroUsize;

// Good - bounded cache
let cache: LruCache<Key, Value> = LruCache::new(NonZeroUsize::new(10_000).unwrap());

// Bad - unbounded, can grow forever
let cache: HashMap<Key, Value> = HashMap::new();
```

## Avoiding Allocations

```rust
// Prefer &str over String in function parameters
fn process(data: &str) { }  // Good
fn process(data: String) { } // Allocates

// Use Cow for conditional ownership
use std::borrow::Cow;
fn normalize(s: &str) -> Cow<str> {
    if needs_change(s) {
        Cow::Owned(transform(s))
    } else {
        Cow::Borrowed(s)
    }
}

// Use SmallVec for typically-small collections
use smallvec::SmallVec;
let items: SmallVec<[Item; 8]> = SmallVec::new();
```

## Async Optimization

```rust
// Use tokio::join! for concurrent independent operations
let (users, posts) = tokio::join!(
    fetch_users(),
    fetch_posts()
);

// Buffer streams for batching
use futures::StreamExt;
stream.chunks(100).for_each_concurrent(4, |batch| async {
    process_batch(batch).await;
});
```

## Memory Profiling

Run memory profiling with DHAT:

```bash
./scripts/memory-profile.sh dhat 30
```

Check for leaks with Valgrind:

```bash
./scripts/memory-profile.sh valgrind 30
```

## Build Profiles

```toml
# Fast compilation for development
[profile.dev]
opt-level = 0

# Maximum optimization for release
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 16

# Maximum optimization, slower compile
[profile.release-fat]
inherits = "release"
lto = "fat"
codegen-units = 1
```

## Profiling Commands

```bash
# CPU flamegraph
cargo flamegraph --release --example server

# Memory profiling
cargo run --example memory_profile_server --features memory-profiling

# Benchmarks
cargo bench -p armature-<crate> --bench <name>

# HTTP load testing
oha -n 10000 -c 100 http://localhost:3000/
```

## Checklist

- [ ] Use `black_box()` in benchmarks
- [ ] Bound all caches with LRU or similar
- [ ] Profile before optimizing
- [ ] Use arena allocation for request data
- [ ] Prefer `&str` over `String` parameters
- [ ] Use `tokio::join!` for concurrent I/O


## Cursor rule: `.cursor/rules/proc-macro-development.mdc`

_Guidelines for developing procedural macros in armature-proc-macro_

Applies to: `armature-proc-macro/**/*.rs`

# Procedural Macro Development

Standards for developing procedural macros in the Armature framework.

## Core Principles

1. **Use syn for parsing, quote for generation**
2. **Preserve span information** for good error messages
3. **Never panic** - use `syn::Error` for compile-time errors
4. **Use fully qualified paths** to avoid conflicts

## Error Handling

```rust
use syn::{Error, Result};

fn validate_input(input: &DeriveInput) -> Result<()> {
    if !matches!(input.data, Data::Struct(_)) {
        return Err(Error::new_spanned(
            input,
            "#[injectable] can only be applied to structs"
        ));
    }
    Ok(())
}
```

## Span Preservation

Always preserve spans for error messages:

```rust
// Good - preserves span
let name = &input.ident;
quote_spanned! {name.span()=>
    impl #name { }
}

// Bad - loses span information
let name_str = input.ident.to_string();
let name = format_ident!("{}", name_str);
```

## Handling Generics

```rust
fn impl_trait(input: &DeriveInput) -> TokenStream {
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    quote! {
        impl #impl_generics MyTrait for #name #ty_generics #where_clause {
            // implementation
        }
    }
}
```

## Hygiene

Prefix generated identifiers to avoid conflicts:

```rust
let field_name = format_ident!("__armature_{}", field.ident.as_ref().unwrap());
```

## Fully Qualified Paths

Always use full paths in generated code:

```rust
quote! {
    // Good
    ::std::result::Result::Ok(())
    ::armature_core::Injectable

    // Bad - could conflict with user's imports
    Result::Ok(())
    Injectable
}
```

## Testing

Use `trybuild` for compile-fail tests:

```rust
#[test]
fn test_compile_errors() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
```

Verify output with `cargo-expand`:

```bash
cargo expand --example my_example
```

## Main Macros Reference

| Macro | Purpose |
|-------|---------|
| `#[injectable]` | Register type with DI container |
| `#[controller]` | Define HTTP controller with route prefix |
| `#[module]` | Define module with providers/controllers |
| `#[get]`, `#[post]`, etc. | HTTP route handlers |
| `#[guard]` | Authorization guard |
| `#[interceptor]` | Request/response interceptor |

## Minimal Generated Code

Keep generated code minimal:

```rust
// Good - minimal wrapper
quote! {
    impl Injectable for #name {
        fn create(container: &Container) -> Self {
            Self::new(container.resolve())
        }
    }
}

// Bad - too much generated code
// Move logic to runtime library instead
```


## Cursor rule: `.cursor/rules/product-manager.mdc`

_Product Manager agent for requirements, roadmap, and feature planning_

Applies to: `docs/**/*.md, **/*.md, .github/**/*.md`

# Product Manager Agent

You are a **Senior Product Manager** for the Armature framework. Your role is to help with product strategy, feature prioritization, requirements gathering, and roadmap planning.

## Your Expertise

- Product strategy and vision
- Feature prioritization (RICE, MoSCoW, Kano)
- User story writing and refinement
- Roadmap planning and communication
- Competitive analysis
- User research synthesis
- Release planning
- Stakeholder management

## Communication Style

- Clear, concise, and actionable
- Focus on user value and business outcomes
- Data-driven recommendations when possible
- Balance technical feasibility with user needs
- Use frameworks and structured thinking

## Key Responsibilities

### 1. Feature Requests

When evaluating feature requests:

```markdown
## Feature Assessment: [Feature Name]

### Problem Statement
What user problem does this solve?

### Target Users
Who benefits from this feature?

### Success Metrics
How will we measure success?

### RICE Score
- **Reach:** How many users affected? (1-10)
- **Impact:** How much will it improve their experience? (0.25, 0.5, 1, 2, 3)
- **Confidence:** How sure are we? (0.5, 0.8, 1.0)
- **Effort:** Person-weeks to implement

**Score:** (Reach × Impact × Confidence) / Effort = X

### Recommendation
[ ] Must Have | [ ] Should Have | [ ] Could Have | [ ] Won't Have
```

### 2. User Stories

Write user stories in this format:

```markdown
## User Story: [Title]

**As a** [type of user]
**I want** [capability/feature]
**So that** [benefit/value]

### Acceptance Criteria
- [ ] Given [context], when [action], then [outcome]
- [ ] Given [context], when [action], then [outcome]

### Technical Notes
- Dependencies: [list]
- Risks: [list]
- Estimated effort: [S/M/L/XL]

### Out of Scope
- [What this story does NOT include]
```

### 3. Roadmap Planning

Structure roadmap items as:

```markdown
## Q[N] [Year] Roadmap

### Theme: [Quarter Theme]

#### Now (Current Sprint)
| Feature | Status | Owner | ETA |
|---------|--------|-------|-----|
| Feature A | In Progress | @dev | Week 2 |

#### Next (Next 2-4 Weeks)
| Feature | Priority | Effort | Dependencies |
|---------|----------|--------|--------------|
| Feature B | P1 | M | Feature A |

#### Later (This Quarter)
| Feature | Priority | Effort | Notes |
|---------|----------|--------|-------|
| Feature C | P2 | L | Needs research |

#### Future (Backlog)
- Feature D - Pending user research
- Feature E - Blocked on upstream
```

### 4. Release Notes

Draft release notes for users:

```markdown
## Armature v[X.Y.Z] Release Notes

**Release Date:** [Date]

### 🚀 New Features
- **[Feature Name]** - Brief description of what users can now do
  - Sub-feature or detail

### 🐛 Bug Fixes
- Fixed issue where [problem] occurred when [action]

### ⚡ Performance Improvements
- [Component] is now X% faster

### 🔧 Breaking Changes
- `old_api()` has been replaced with `new_api()`
  - Migration: [steps]

### 📚 Documentation
- Added guide for [topic]

### 🙏 Contributors
Thanks to @contributor1, @contributor2 for their contributions!
```

### 5. Competitive Analysis

When analyzing competitors:

```markdown
## Competitive Analysis: [Competitor]

### Overview
Brief description of the competitor

### Feature Comparison
| Feature | Armature | Competitor | Notes |
|---------|----------|------------|-------|
| Feature A | ✅ | ✅ | Parity |
| Feature B | ✅ | ❌ | Our advantage |
| Feature C | ❌ | ✅ | Gap to address |

### Strengths
- [List competitor strengths]

### Weaknesses
- [List competitor weaknesses]

### Opportunities for Armature
- [How we can differentiate]

### Threats
- [Risks to be aware of]
```

## Framework-Specific Context

### Armature's Value Proposition
- Angular/NestJS-inspired Rust web framework
- Decorator-based API with procedural macros
- Built-in dependency injection
- Type-safe, high-performance
- Batteries-included approach

### Target Users
1. **Rust developers** building web APIs
2. **TypeScript/NestJS developers** transitioning to Rust
3. **Enterprise teams** needing type-safe backends

### Key Differentiators vs Competitors
| vs Actix-web | vs Axum | vs Rocket |
|--------------|---------|-----------|
| Higher-level abstractions | More opinionated | Stable, async-first |
| Built-in DI | Decorator syntax | Better DX |
| NestJS familiarity | Full-featured | Production-ready |

## Interaction Guidelines

When asked to help with product work:

1. **Clarify the goal** - Understand what outcome is desired
2. **Gather context** - Ask about users, constraints, timeline
3. **Provide structure** - Use appropriate frameworks/templates
4. **Prioritize ruthlessly** - Focus on highest-impact items
5. **Consider trade-offs** - Technical debt vs speed, scope vs quality
6. **Document decisions** - Capture rationale for future reference

## Example Prompts I Can Help With

- "Help me prioritize these feature requests"
- "Write user stories for the new caching feature"
- "Create a roadmap for Q1"
- "Draft release notes for v0.5.0"
- "Compare Armature to Actix-web"
- "What should we build next?"
- "Help me scope this feature"
- "Create acceptance criteria for this story"


## Cursor rule: `.cursor/rules/queue-background-jobs.mdc`

_Guidelines for background job processing with armature-queue_

Applies to: `armature-queue/**/*.rs, "**/jobs/**/*.rs, **/workers/**/*.rs`

# Queue & Background Jobs

Standards for background job processing in Armature.

## Job Definition

```rust
use armature_queue::{Job, JobContext, JobResult};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct SendEmailJob {
    pub to: String,
    pub subject: String,
    pub body: String,
}

impl Job for SendEmailJob {
    const NAME: &'static str = "send_email";
    const QUEUE: &'static str = "emails";
    const MAX_RETRIES: u32 = 3;
    const TIMEOUT: Duration = Duration::from_secs(30);

    async fn perform(&self, ctx: &JobContext) -> JobResult {
        let mailer = ctx.resolve::<Mailer>()?;

        mailer.send(&self.to, &self.subject, &self.body).await?;

        Ok(())
    }
}
```

## Enqueueing Jobs

```rust
// Immediate execution
queue.enqueue(SendEmailJob {
    to: "user@example.com".into(),
    subject: "Welcome!".into(),
    body: "Hello, welcome to our app.".into(),
}).await?;

// Delayed execution
queue.enqueue_at(
    SendEmailJob { /* ... */ },
    Utc::now() + Duration::hours(1),
).await?;

// With priority
queue.enqueue_with_priority(
    SendEmailJob { /* ... */ },
    Priority::High,
).await?;
```

## Worker Configuration

```rust
let worker = Worker::new(queue)
    .concurrency(4)
    .queues(&["critical", "default", "low"])
    .register::<SendEmailJob>()
    .register::<ProcessImageJob>()
    .register::<GenerateReportJob>();

// Start processing
worker.run().await?;
```

## Retry Strategy

```rust
impl Job for SendEmailJob {
    fn retry_delay(&self, attempt: u32) -> Duration {
        // Exponential backoff: 10s, 60s, 360s, ...
        Duration::from_secs(10 * 6u64.pow(attempt - 1))
    }

    fn should_retry(&self, error: &JobError) -> bool {
        // Don't retry permanent failures
        !matches!(error, JobError::InvalidEmail(_))
    }
}
```

## Error Handling

```rust
#[derive(Error, Debug)]
pub enum JobError {
    #[error("Temporary failure: {0}")]
    Temporary(String), // Will retry

    #[error("Permanent failure: {0}")]
    Permanent(String), // Won't retry
}

async fn perform(&self, ctx: &JobContext) -> JobResult {
    match send_email(&self.to).await {
        Ok(()) => Ok(()),
        Err(e) if e.is_transient() => Err(JobError::Temporary(e.to_string()).into()),
        Err(e) => Err(JobError::Permanent(e.to_string()).into()),
    }
}
```

## Job Lifecycle Hooks

```rust
impl Job for ProcessImageJob {
    async fn before_perform(&self, ctx: &JobContext) -> Result<(), JobError> {
        tracing::info!(job_id = %ctx.job_id, "Starting image processing");
        Ok(())
    }

    async fn after_perform(&self, ctx: &JobContext, result: &JobResult) {
        match result {
            Ok(()) => tracing::info!(job_id = %ctx.job_id, "Image processed"),
            Err(e) => tracing::error!(job_id = %ctx.job_id, error = %e, "Processing failed"),
        }
    }

    async fn on_failure(&self, ctx: &JobContext, error: &JobError) {
        // Send alert, update status, etc.
        alert_team(format!("Job {} failed: {}", ctx.job_id, error)).await;
    }
}
```

## Batch Jobs

```rust
// Enqueue multiple jobs atomically
queue.enqueue_batch(vec![
    SendEmailJob { to: "user1@example.com".into(), /* ... */ },
    SendEmailJob { to: "user2@example.com".into(), /* ... */ },
    SendEmailJob { to: "user3@example.com".into(), /* ... */ },
]).await?;
```

## Scheduled Jobs (Cron)

```rust
use armature_cron::Schedule;

scheduler
    .add("cleanup_expired_sessions", "0 0 * * *", || async {
        cleanup_sessions().await
    })
    .add("generate_daily_report", "0 8 * * *", || async {
        generate_report().await
    })
    .start()
    .await?;
```

## Monitoring

```rust
// Get queue statistics
let stats = queue.stats().await?;
println!("Pending: {}", stats.pending);
println!("Processing: {}", stats.processing);
println!("Failed: {}", stats.failed);
println!("Completed: {}", stats.completed);

// Dead letter queue
let dead_jobs = queue.dead_letter_queue().list(100).await?;
for job in dead_jobs {
    // Inspect or retry
    queue.retry_dead_job(job.id).await?;
}
```

## Testing Jobs

```rust
#[tokio::test]
async fn test_send_email_job() {
    let mock_mailer = MockMailer::new();
    mock_mailer.expect_send().times(1).returning(|_, _, _| Ok(()));

    let ctx = JobContext::test()
        .with_service::<dyn Mailer>(Arc::new(mock_mailer));

    let job = SendEmailJob {
        to: "test@example.com".into(),
        subject: "Test".into(),
        body: "Body".into(),
    };

    let result = job.perform(&ctx).await;
    assert!(result.is_ok());
}
```


## Cursor rule: `.cursor/rules/realtime-websocket-sse.mdc`

# Real-time Features: WebSocket & SSE

Guidelines for implementing real-time communication in Armature.

## Choosing Between WebSocket and SSE

| Feature | WebSocket | SSE |
|---------|-----------|-----|
| Direction | Bidirectional | Server → Client only |
| Protocol | Custom | HTTP |
| Reconnection | Manual | Automatic |
| Browser Support | Good | Excellent |
| Proxy Friendly | Sometimes | Yes |
| Use Case | Chat, Games | Notifications, Feeds |

## WebSocket Implementation

### Basic WebSocket Handler

```rust
use armature::websocket::{WebSocket, Message, WebSocketUpgrade};

#[controller("/ws")]
pub struct WebSocketController;

impl WebSocketController {
    #[get("/")]
    async fn connect(&self, ws: WebSocketUpgrade) -> WebSocketResponse {
        ws.on_upgrade(|socket| handle_socket(socket))
    }
}

async fn handle_socket(mut socket: WebSocket) {
    // Send welcome message
    socket.send(Message::Text("Connected!".to_string())).await.ok();

    // Message loop
    while let Some(msg) = socket.recv().await {
        match msg {
            Ok(Message::Text(text)) => {
                // Echo back
                socket.send(Message::Text(format!("Echo: {}", text))).await.ok();
            }
            Ok(Message::Binary(data)) => {
                // Handle binary data
                socket.send(Message::Binary(data)).await.ok();
            }
            Ok(Message::Ping(data)) => {
                socket.send(Message::Pong(data)).await.ok();
            }
            Ok(Message::Close(_)) => break,
            Err(e) => {
                eprintln!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }
}
```

### Room-Based Chat

```rust
use std::sync::Arc;
use tokio::sync::{broadcast, RwLock};
use std::collections::HashMap;

#[derive(Clone)]
pub struct ChatRooms {
    rooms: Arc<RwLock<HashMap<String, broadcast::Sender<ChatMessage>>>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub room: String,
    pub user: String,
    pub content: String,
    pub timestamp: u64,
}

impl ChatRooms {
    pub fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn join(&self, room: &str) -> broadcast::Receiver<ChatMessage> {
        let mut rooms = self.rooms.write().await;

        if let Some(tx) = rooms.get(room) {
            tx.subscribe()
        } else {
            let (tx, rx) = broadcast::channel(100);
            rooms.insert(room.to_string(), tx);
            rx
        }
    }

    pub async fn send(&self, message: ChatMessage) -> Result<(), Error> {
        let rooms = self.rooms.read().await;

        if let Some(tx) = rooms.get(&message.room) {
            tx.send(message).map_err(|_| Error::ChannelClosed)?;
        }

        Ok(())
    }

    pub async fn leave(&self, room: &str) {
        let mut rooms = self.rooms.write().await;

        if let Some(tx) = rooms.get(room) {
            if tx.receiver_count() == 0 {
                rooms.remove(room);
            }
        }
    }
}

#[controller("/ws/chat")]
pub struct ChatController {
    rooms: ChatRooms,
}

impl ChatController {
    #[get("/:room")]
    async fn join_room(
        &self,
        room: Path<String>,
        user: Query<UserQuery>,
        ws: WebSocketUpgrade,
    ) -> WebSocketResponse {
        let rooms = self.rooms.clone();
        let room_name = room.to_string();
        let username = user.name.clone();

        ws.on_upgrade(move |socket| async move {
            handle_chat_socket(socket, rooms, room_name, username).await
        })
    }
}

async fn handle_chat_socket(
    socket: WebSocket,
    rooms: ChatRooms,
    room: String,
    user: String,
) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = rooms.join(&room).await;

    // Announce join
    let join_msg = ChatMessage {
        room: room.clone(),
        user: "system".to_string(),
        content: format!("{} joined the room", user),
        timestamp: timestamp_now(),
    };
    rooms.send(join_msg).await.ok();

    // Spawn task to forward broadcast messages to client
    let room_clone = room.clone();
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            if sender.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
    });

    // Receive messages from client
    let rooms_clone = rooms.clone();
    let user_clone = user.clone();
    let room_clone2 = room.clone();
    let recv_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(text))) = receiver.next().await {
            let msg = ChatMessage {
                room: room_clone2.clone(),
                user: user_clone.clone(),
                content: text,
                timestamp: timestamp_now(),
            };
            rooms_clone.send(msg).await.ok();
        }
    });

    // Wait for either task to complete
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }

    // Announce leave
    let leave_msg = ChatMessage {
        room: room.clone(),
        user: "system".to_string(),
        content: format!("{} left the room", user),
        timestamp: timestamp_now(),
    };
    rooms.send(leave_msg).await.ok();
    rooms.leave(&room).await;
}
```

### WebSocket with Authentication

```rust
use armature::auth::JwtService;

#[controller("/ws")]
pub struct AuthenticatedWebSocket {
    jwt: JwtService,
}

impl AuthenticatedWebSocket {
    #[get("/")]
    async fn connect(
        &self,
        token: Query<TokenQuery>,
        ws: WebSocketUpgrade,
    ) -> Result<WebSocketResponse, Error> {
        // Validate token before upgrading
        let claims = self.jwt.verify(&token.token)?;

        Ok(ws.on_upgrade(move |socket| async move {
            handle_authenticated_socket(socket, claims).await
        }))
    }
}

async fn handle_authenticated_socket(mut socket: WebSocket, claims: Claims) {
    // User is authenticated
    socket.send(Message::Text(format!(
        "Welcome, {}!", claims.sub
    ))).await.ok();

    // ... handle messages
}
```

## Server-Sent Events (SSE)

### Basic SSE Stream

```rust
use armature::sse::{Sse, Event, KeepAlive};
use tokio_stream::StreamExt;

#[controller("/events")]
pub struct EventController;

impl EventController {
    #[get("/")]
    async fn stream(&self) -> Sse<impl Stream<Item = Result<Event, Error>>> {
        let stream = async_stream::stream! {
            let mut interval = tokio::time::interval(Duration::from_secs(1));

            loop {
                interval.tick().await;
                let event = Event::default()
                    .data(format!("Server time: {}", timestamp_now()));
                yield Ok(event);
            }
        };

        Sse::new(stream)
            .keep_alive(KeepAlive::default())
    }
}
```

### Named Events with Data

```rust
#[derive(Serialize)]
struct Notification {
    id: String,
    title: String,
    body: String,
    created_at: u64,
}

#[get("/notifications")]
async fn notification_stream(&self, user_id: Auth<UserId>) -> Sse<impl Stream<Item = Result<Event, Error>>> {
    let user_id = user_id.0;
    let notifications = self.notification_service.subscribe(user_id).await;

    let stream = notifications.map(|notification| {
        let json = serde_json::to_string(&notification)?;
        Ok(Event::default()
            .event("notification")
            .data(json)
            .id(notification.id.clone()))
    });

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive")
        )
}
```

### SSE with Reconnection

```rust
#[get("/feed")]
async fn feed_stream(
    &self,
    last_event_id: Header<"Last-Event-Id">,
) -> Sse<impl Stream<Item = Result<Event, Error>>> {
    // Resume from last event ID if reconnecting
    let start_from = last_event_id
        .as_deref()
        .and_then(|id| id.parse::<u64>().ok())
        .unwrap_or(0);

    let stream = self.feed_service.stream_from(start_from).await
        .map(|item| {
            Ok(Event::default()
                .event("feed-item")
                .data(serde_json::to_string(&item)?)
                .id(item.id.to_string())
                .retry(Duration::from_secs(5)))
        });

    Sse::new(stream)
}
```

## Broadcasting Patterns

### Pub/Sub with Tokio Broadcast

```rust
use tokio::sync::broadcast;

pub struct EventBroadcaster {
    tx: broadcast::Sender<ServerEvent>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type")]
pub enum ServerEvent {
    UserOnline { user_id: i64 },
    UserOffline { user_id: i64 },
    NewMessage { from: i64, content: String },
    SystemNotice { message: String },
}

impl EventBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(1000);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.tx.subscribe()
    }

    pub fn broadcast(&self, event: ServerEvent) {
        // Ignore send errors (no subscribers)
        let _ = self.tx.send(event);
    }
}

#[controller("/events")]
pub struct BroadcastController {
    broadcaster: EventBroadcaster,
}

impl BroadcastController {
    #[get("/stream")]
    async fn stream(&self) -> Sse<impl Stream<Item = Result<Event, Error>>> {
        let mut rx = self.broadcaster.subscribe();

        let stream = async_stream::stream! {
            while let Ok(event) = rx.recv().await {
                let json = serde_json::to_string(&event)?;
                yield Ok(Event::default()
                    .event(event.event_type())
                    .data(json));
            }
        };

        Sse::new(stream)
    }
}
```

### Channel per Client

```rust
use tokio::sync::mpsc;

pub struct ClientManager {
    clients: Arc<RwLock<HashMap<ClientId, mpsc::Sender<ServerEvent>>>>,
}

impl ClientManager {
    pub async fn add_client(&self) -> (ClientId, mpsc::Receiver<ServerEvent>) {
        let id = ClientId::new();
        let (tx, rx) = mpsc::channel(100);

        self.clients.write().await.insert(id.clone(), tx);
        (id, rx)
    }

    pub async fn remove_client(&self, id: &ClientId) {
        self.clients.write().await.remove(id);
    }

    pub async fn send_to(&self, id: &ClientId, event: ServerEvent) -> Result<(), Error> {
        let clients = self.clients.read().await;
        if let Some(tx) = clients.get(id) {
            tx.send(event).await.map_err(|_| Error::ClientDisconnected)?;
        }
        Ok(())
    }

    pub async fn broadcast(&self, event: ServerEvent) {
        let clients = self.clients.read().await;
        for tx in clients.values() {
            let _ = tx.send(event.clone()).await;
        }
    }
}
```

## Connection Management

### Heartbeat/Ping-Pong

```rust
async fn handle_socket_with_heartbeat(mut socket: WebSocket) {
    let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(30));
    let mut last_pong = Instant::now();

    loop {
        tokio::select! {
            _ = heartbeat_interval.tick() => {
                // Check if client is still alive
                if last_pong.elapsed() > Duration::from_secs(60) {
                    println!("Client timeout, disconnecting");
                    break;
                }

                // Send ping
                if socket.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Pong(_))) => {
                        last_pong = Instant::now();
                    }
                    Some(Ok(Message::Text(text))) => {
                        // Handle message
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}
```

### Connection Limits

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct ConnectionLimiter {
    current: AtomicUsize,
    max: usize,
}

impl ConnectionLimiter {
    pub fn new(max: usize) -> Self {
        Self {
            current: AtomicUsize::new(0),
            max,
        }
    }

    pub fn try_acquire(&self) -> Option<ConnectionGuard> {
        loop {
            let current = self.current.load(Ordering::SeqCst);
            if current >= self.max {
                return None;
            }

            if self.current.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ).is_ok() {
                return Some(ConnectionGuard { limiter: self });
            }
        }
    }
}

pub struct ConnectionGuard<'a> {
    limiter: &'a ConnectionLimiter,
}

impl Drop for ConnectionGuard<'_> {
    fn drop(&mut self) {
        self.limiter.current.fetch_sub(1, Ordering::SeqCst);
    }
}
```

## Client-Side Examples

### JavaScript WebSocket Client

```javascript
class WebSocketClient {
  constructor(url) {
    this.url = url;
    this.reconnectAttempts = 0;
    this.maxReconnectAttempts = 5;
    this.connect();
  }

  connect() {
    this.ws = new WebSocket(this.url);

    this.ws.onopen = () => {
      console.log('Connected');
      this.reconnectAttempts = 0;
    };

    this.ws.onmessage = (event) => {
      const data = JSON.parse(event.data);
      this.handleMessage(data);
    };

    this.ws.onclose = () => {
      console.log('Disconnected');
      this.reconnect();
    };

    this.ws.onerror = (error) => {
      console.error('WebSocket error:', error);
    };
  }

  reconnect() {
    if (this.reconnectAttempts < this.maxReconnectAttempts) {
      this.reconnectAttempts++;
      const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30000);
      setTimeout(() => this.connect(), delay);
    }
  }

  send(data) {
    if (this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(data));
    }
  }

  handleMessage(data) {
    // Override in subclass
  }
}
```

### JavaScript SSE Client

```javascript
class EventSourceClient {
  constructor(url) {
    this.eventSource = new EventSource(url);

    this.eventSource.onopen = () => {
      console.log('SSE connected');
    };

    this.eventSource.onerror = (error) => {
      console.error('SSE error:', error);
    };

    // Listen for specific events
    this.eventSource.addEventListener('notification', (event) => {
      const data = JSON.parse(event.data);
      this.handleNotification(data);
    });

    // Default message handler
    this.eventSource.onmessage = (event) => {
      console.log('Message:', event.data);
    };
  }

  handleNotification(data) {
    // Override in subclass
  }

  close() {
    this.eventSource.close();
  }
}
```

## Best Practices

1. **Use SSE for server-to-client only** - Simpler, automatic reconnection
2. **Use WebSocket for bidirectional** - Chat, games, collaboration
3. **Implement heartbeats** for WebSocket connections
4. **Handle reconnection** gracefully on both sides
5. **Limit connections** per client/IP
6. **Authenticate before upgrade** for WebSocket
7. **Use event IDs** for SSE to enable resume
8. **Set appropriate timeouts** for idle connections
9. **Broadcast efficiently** with channels
10. **Clean up** resources on disconnect

## Summary

| Pattern | WebSocket | SSE |
|---------|-----------|-----|
| Chat | ✅ Best | ❌ |
| Notifications | ⚠️ Works | ✅ Best |
| Live Feed | ⚠️ Works | ✅ Best |
| Gaming | ✅ Best | ❌ |
| Collaboration | ✅ Best | ❌ |
| Progress Updates | ⚠️ Works | ✅ Best |


## Cursor rule: `.cursor/rules/rust-2024-best-practices.mdc`

# Rust 2024 Best Practices

This project uses **Rust Edition 2024** and follows modern Rust best practices.

## Edition Configuration

All `Cargo.toml` files must specify edition 2024:

```toml
[package]
name = "crate-name"
version = "0.1.0"
edition = "2024"
```

## Code Style

### Use `rustfmt`

Always format code with rustfmt before committing:

```bash
cargo fmt --all
```

### Use `clippy`

Run clippy and fix all warnings:

```bash
cargo clippy --all-features --all-targets -- -D warnings
```

### Naming Conventions

```rust
// ✅ Good
struct UserAccount { }           // Types: UpperCamelCase
trait Validate { }               // Traits: UpperCamelCase
enum HttpStatus { }              // Enums: UpperCamelCase
fn create_user() { }             // Functions: snake_case
const MAX_SIZE: usize = 100;     // Constants: SCREAMING_SNAKE_CASE
static GLOBAL_CONFIG: &str = ""; // Statics: SCREAMING_SNAKE_CASE
let user_name = "Alice";         // Variables: snake_case
mod http_client;                 // Modules: snake_case

// ❌ Bad
struct user_account { }          // Wrong case
fn CreateUser() { }              // Wrong case
const maxSize: usize = 100;      // Wrong case
```

## Error Handling

### Use `Result` and `?` Operator

```rust
// ✅ Good: Use Result and ? operator
pub fn parse_config(path: &str) -> Result<Config, Error> {
    let contents = std::fs::read_to_string(path)?;
    let config: Config = serde_json::from_str(&contents)?;
    validate_config(&config)?;
    Ok(config)
}

// ❌ Bad: Using unwrap in library code
pub fn parse_config(path: &str) -> Config {
    let contents = std::fs::read_to_string(path).unwrap(); // Don't panic!
    serde_json::from_str(&contents).unwrap()
}
```

### Use `thiserror` for Error Types

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CacheError {
    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Cache key not found: {0}")]
    NotFound(String),
}
```

### Never Use `unwrap()` or `expect()` in Library Code

```rust
// ✅ Good: Propagate errors
pub fn get_user(id: i32) -> Result<User, Error> {
    database.query("SELECT * FROM users WHERE id = ?", id)?
        .ok_or_else(|| Error::NotFound(format!("User {} not found", id)))
}

// ❌ Bad: Panicking in library code
pub fn get_user(id: i32) -> User {
    database.query("SELECT * FROM users WHERE id = ?", id)
        .unwrap()
        .expect("User not found") // Never use in library code!
}

// ✅ Acceptable: In application code, example code, or tests
#[tokio::main]
async fn main() {
    let app = Application::create(AppModule);
    app.listen(3000).await.expect("Failed to start server");
}
```

## Async/Await

### Use `async-trait` for Async Traits

```rust
use async_trait::async_trait;

#[async_trait]
pub trait Repository: Send + Sync {
    async fn find_by_id(&self, id: i32) -> Result<User, Error>;
    async fn save(&self, user: &User) -> Result<(), Error>;
}
```

### Prefer `tokio` Runtime

```rust
// ✅ Good: Use tokio for async runtime
#[tokio::main]
async fn main() -> Result<(), Error> {
    // Async code
    Ok(())
}

// For tests
#[tokio::test]
async fn test_async_function() {
    let result = async_function().await;
    assert!(result.is_ok());
}
```

### Use `.await` Properly

```rust
// ✅ Good: Await futures properly
let user = fetch_user(id).await?;
let posts = fetch_posts(user.id).await?;

// ✅ Good: Concurrent execution with join!
let (user, posts) = tokio::join!(
    fetch_user(id),
    fetch_posts(user_id)
);

// ❌ Bad: Blocking in async code
async fn bad_example() {
    std::thread::sleep(Duration::from_secs(1)); // Blocks entire thread!
}

// ✅ Good: Async sleep
async fn good_example() {
    tokio::time::sleep(Duration::from_secs(1)).await;
}
```

## Ownership and Borrowing

### Follow Ownership Rules

```rust
// ✅ Good: Clear ownership
pub struct User {
    pub id: i32,
    pub name: String,
}

impl User {
    // Takes ownership
    pub fn new(name: String) -> Self {
        Self { id: 0, name }
    }

    // Borrows immutably
    pub fn display(&self) {
        println!("{}", self.name);
    }

    // Borrows mutably
    pub fn update_name(&mut self, name: String) {
        self.name = name;
    }

    // Consumes self
    pub fn into_dto(self) -> UserDto {
        UserDto { name: self.name }
    }
}
```

### Use References Appropriately

```rust
// ✅ Good: Take references for read-only access
pub fn validate_email(email: &str) -> bool {
    email.contains('@')
}

// ❌ Bad: Unnecessary ownership
pub fn validate_email(email: String) -> bool {
    email.contains('@')
}

// ✅ Good: Return owned data
pub fn format_name(first: &str, last: &str) -> String {
    format!("{} {}", first, last)
}
```

### Prefer `&str` Over `&String`

```rust
// ✅ Good: Use &str for string parameters
pub fn process_text(text: &str) -> String {
    text.to_uppercase()
}

// ❌ Bad: Unnecessarily specific
pub fn process_text(text: &String) -> String {
    text.to_uppercase()
}
```

## Type Safety

### Use Newtypes for Type Safety

```rust
// ✅ Good: Type-safe wrappers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserId(pub i32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostId(pub i32);

// Now you can't accidentally mix them up
fn get_user(id: UserId) -> Result<User, Error> { }
fn get_post(id: PostId) -> Result<Post, Error> { }

// ❌ Bad: Easy to mix up IDs
fn get_user(id: i32) -> Result<User, Error> { }
fn get_post(id: i32) -> Result<Post, Error> { }
```

### Use Enums for State

```rust
// ✅ Good: Type-safe state machine
pub enum PaymentStatus {
    Pending { amount: f64 },
    Processing { transaction_id: String },
    Completed { receipt: String },
    Failed { error: String },
}

impl PaymentStatus {
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Failed { .. })
    }
}
```

### Leverage the Type System

```rust
// ✅ Good: Use type system to prevent invalid states
pub struct ValidatedEmail(String);

impl ValidatedEmail {
    pub fn new(email: String) -> Result<Self, Error> {
        if email.contains('@') {
            Ok(Self(email))
        } else {
            Err(Error::InvalidEmail)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// Now you can't create invalid emails
fn send_email(to: ValidatedEmail, body: &str) { }
```

## Pattern Matching

### Use Exhaustive Pattern Matching

```rust
// ✅ Good: Exhaustive matching
match status {
    HttpStatus::Ok => handle_success(),
    HttpStatus::NotFound => handle_not_found(),
    HttpStatus::ServerError => handle_error(),
    // Compiler ensures all cases are covered
}

// ⚠️ Use _ sparingly and intentionally
match status {
    HttpStatus::Ok => handle_success(),
    _ => handle_other(), // Only if appropriate
}
```

### Use `if let` for Single Pattern

```rust
// ✅ Good: Use if let for single pattern
if let Some(user) = find_user(id) {
    process_user(user);
}

// ❌ Bad: Unnecessary match for single pattern
match find_user(id) {
    Some(user) => process_user(user),
    None => {}
}

// ✅ Good: Use let-else for early return (Rust 2021+)
let Some(user) = find_user(id) else {
    return Err(Error::NotFound);
};
```

### Use `matches!` Macro

```rust
// ✅ Good: Use matches! for boolean checks
if matches!(status, HttpStatus::Ok | HttpStatus::Created) {
    // Handle success cases
}

// ❌ Bad: Verbose pattern matching
if match status {
    HttpStatus::Ok | HttpStatus::Created => true,
    _ => false,
} {
    // Handle success cases
}
```

## Iterators

### Prefer Iterators Over Loops

```rust
// ✅ Good: Use iterators
let even_squares: Vec<i32> = numbers
    .iter()
    .filter(|n| n % 2 == 0)
    .map(|n| n * n)
    .collect();

// ❌ Bad: Manual loop
let mut even_squares = Vec::new();
for n in &numbers {
    if n % 2 == 0 {
        even_squares.push(n * n);
    }
}

// ✅ Good: Use iterator methods
let sum: i32 = numbers.iter().sum();
let max = numbers.iter().max();
let found = numbers.iter().find(|&&n| n > 10);
```

### Avoid Unnecessary `collect()`

```rust
// ✅ Good: Chain iterators
let result = data
    .iter()
    .filter(|x| x.is_valid())
    .map(|x| x.process())
    .collect();

// ❌ Bad: Multiple collects
let filtered = data.iter().filter(|x| x.is_valid()).collect::<Vec<_>>();
let result = filtered.iter().map(|x| x.process()).collect();
```

## Safety

### Avoid `unsafe` Unless Necessary

```rust
// ✅ Good: Safe Rust is preferred
pub fn get_slice(data: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    data.get(start..start + len)
}

// ❌ Bad: Unnecessary unsafe
pub fn get_slice(data: &[u8], start: usize, len: usize) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(data.as_ptr().add(start), len)
    }
}
```

### Document `unsafe` Code

```rust
// ✅ Good: Document safety invariants
/// # Safety
///
/// Caller must ensure that:
/// - `ptr` is valid and properly aligned
/// - `ptr` points to `len` initialized elements
/// - The memory is not accessed mutably elsewhere
pub unsafe fn from_raw_parts(ptr: *const T, len: usize) -> &[T] {
    std::slice::from_raw_parts(ptr, len)
}
```

## Performance

### Use `&[T]` Instead of `&Vec<T>`

```rust
// ✅ Good: More flexible
pub fn process_items(items: &[Item]) -> Result<(), Error> {
    for item in items {
        process(item)?;
    }
    Ok(())
}

// ❌ Bad: Too specific
pub fn process_items(items: &Vec<Item>) -> Result<(), Error> {
    // Same code
}
```

### Avoid Cloning When Not Needed

```rust
// ✅ Good: Borrow when possible
pub fn format_user(user: &User) -> String {
    format!("{} ({})", user.name, user.email)
}

// ❌ Bad: Unnecessary clone
pub fn format_user(user: User) -> String {
    format!("{} ({})", user.name, user.email)
}

// ✅ Good: Clone only when necessary
pub fn cache_user(&mut self, user: User) {
    self.cache.insert(user.id, user); // Needs ownership
}
```

### Use `Cow` for Conditional Ownership

```rust
use std::borrow::Cow;

// ✅ Good: Avoid allocation when possible
pub fn ensure_prefix(s: &str, prefix: &str) -> Cow<str> {
    if s.starts_with(prefix) {
        Cow::Borrowed(s)
    } else {
        Cow::Owned(format!("{}{}", prefix, s))
    }
}
```

## Traits

### Implement Standard Traits

```rust
// ✅ Good: Derive common traits
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UserId(pub i32);

// Implement Display for user-facing output
impl std::fmt::Display for UserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "User({})", self.0)
    }
}

// Implement From for conversions
impl From<i32> for UserId {
    fn from(id: i32) -> Self {
        Self(id)
    }
}
```

### Use Trait Bounds Wisely

```rust
// ✅ Good: Clear trait bounds
pub fn serialize<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value)
        .map_err(|e| Error::Serialization(e.to_string()))
}

// ✅ Good: Use where clause for complex bounds
pub fn process<T>(data: T) -> Result<Output, Error>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
{
    // Implementation
}
```

## Documentation

### Document Public APIs

```rust
/// Validates a user email address.
///
/// # Arguments
///
/// * `email` - The email address to validate
///
/// # Returns
///
/// Returns `true` if the email is valid, `false` otherwise.
///
/// # Examples
///
/// ```
/// use armature::validation::validate_email;
///
/// assert!(validate_email("user@example.com"));
/// assert!(!validate_email("invalid"));
/// ```
pub fn validate_email(email: &str) -> bool {
    email.contains('@') && email.contains('.')
}
```

### Document Module Purpose

```rust
//! User authentication and authorization module.
//!
//! This module provides types and functions for handling user
//! authentication, including password hashing, token generation,
//! and permission checking.
//!
//! # Examples
//!
//! ```
//! use armature::auth::*;
//!
//! let auth_service = AuthService::new();
//! let user = auth_service.authenticate(credentials).await?;
//! ```
```

## Summary

1. **Use Rust 2024 edition** in all Cargo.toml files
2. **Run `rustfmt` and `clippy`** before committing
3. **Use `Result` and `?`** for error handling, avoid `unwrap()`
4. **Prefer borrowing** over cloning
5. **Use iterators** over manual loops
6. **Leverage the type system** for safety
7. **Document public APIs** thoroughly
8. **Write comprehensive tests**
9. **Follow naming conventions** strictly
10. **Avoid `unsafe`** unless absolutely necessary

When in doubt, follow the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)!


## Cursor rule: `.cursor/rules/rust-module-creation.mdc`

_Guidelines for creating new armature-* crates following framework conventions_

Applies to: `armature-*/src/**/*.rs, armature-*/Cargo.toml`

# Rust Module Creation

When creating a new `armature-*` crate for the framework, follow these conventions.

## Directory Structure

```
armature-<name>/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Public API exports
│   ├── config.rs       # Builder pattern configuration
│   ├── error.rs        # thiserror-based error types
│   └── <impl>.rs       # Implementation files
└── tests/
    └── integration.rs  # Integration tests
```

## Cargo.toml Template

```toml
[package]
name = "armature-<name>"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
description = "Description of the module"

[dependencies]
armature-core = { path = "../armature-core", optional = true }
thiserror = "2"
tokio = { version = "1", features = ["rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }

[features]
default = []
di = ["armature-core"]
```

## Error Handling

Use `thiserror` for all error types:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MyModuleError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Connection failed: {0}")]
    Connection(#[from] std::io::Error),
}
```

## Configuration Builder Pattern

```rust
#[derive(Debug, Clone)]
pub struct MyModuleConfig {
    pub setting: String,
    pub timeout_ms: u64,
}

impl Default for MyModuleConfig {
    fn default() -> Self {
        Self {
            setting: String::new(),
            timeout_ms: 5000,
        }
    }
}

impl MyModuleConfig {
    pub fn builder() -> MyModuleConfigBuilder {
        MyModuleConfigBuilder::default()
    }
}

#[derive(Default)]
pub struct MyModuleConfigBuilder {
    config: MyModuleConfig,
}

impl MyModuleConfigBuilder {
    pub fn setting(mut self, value: impl Into<String>) -> Self {
        self.config.setting = value.into();
        self
    }

    pub fn build(self) -> MyModuleConfig {
        self.config
    }
}
```

## Dependency Injection Integration

When the `di` feature is enabled:

```rust
#[cfg(feature = "di")]
use armature_core::injectable;

#[cfg_attr(feature = "di", injectable)]
pub struct MyService {
    config: MyModuleConfig,
}
```

## Checklist

- [ ] Add crate to workspace members in root `Cargo.toml`
- [ ] Implement `Default` for config structs
- [ ] Use `thiserror` for error types
- [ ] Add `#[injectable]` when `di` feature is enabled
- [ ] Write doc comments with examples
- [ ] Create integration tests


## Cursor rule: `.cursor/rules/security-authentication.mdc`

_Security and authentication standards for Armature applications_

Applies to: `armature-auth/**/*.rs, armature-oauth2/**/*.rs, armature-saml/**/*.rs, **/auth/**/*.rs`

# Security & Authentication

Security standards for authentication, authorization, and secure coding.

## Password Hashing

Use Argon2id (preferred) or bcrypt:

```rust
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use argon2::password_hash::{SaltString, rand_core::OsRng};

pub fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();

    Ok(argon2
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, AuthError> {
    let parsed_hash = PasswordHash::new(hash)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}
```

## JWT Configuration

```rust
// Use RS256 for production, HS256 only for development
pub struct JwtConfig {
    pub algorithm: Algorithm,      // RS256 preferred
    pub access_ttl: Duration,      // 15 minutes max
    pub refresh_ttl: Duration,     // 7 days max
    pub issuer: String,
    pub audience: Vec<String>,
}

// Always validate claims
fn validate_token(token: &str) -> Result<Claims, AuthError> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&["my-app"]);
    validation.set_issuer(&["my-issuer"]);
    validation.validate_exp = true;
    validation.validate_nbf = true;

    decode::<Claims>(token, &key, &validation)
}
```

## OAuth2 / PKCE

Always use PKCE for public clients:

```rust
use pkce::{CodeChallenge, CodeVerifier};

let verifier = CodeVerifier::new();
let challenge = CodeChallenge::from_verifier(&verifier);

// Store verifier in session, send challenge to authorization server
```

## Authorization Guards

```rust
#[injectable]
pub struct RoleGuard {
    required_roles: Vec<String>,
}

impl Guard for RoleGuard {
    async fn can_activate(&self, ctx: &RequestContext) -> Result<bool, GuardError> {
        let user = ctx.get::<AuthenticatedUser>()?;

        let has_role = self.required_roles
            .iter()
            .any(|role| user.roles.contains(role));

        if !has_role {
            // Log authorization failure
            tracing::warn!(
                user_id = %user.id,
                required = ?self.required_roles,
                "Authorization denied"
            );
        }

        Ok(has_role)
    }
}
```

## Input Validation

```rust
use validator::Validate;

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8, max = 128))]
    pub password: String,
}

// Always validate before processing
async fn login(req: Json<LoginRequest>) -> Result<Response, Error> {
    req.validate()?;
    // ...
}
```

## Security Headers

```rust
fn security_headers() -> impl Middleware {
    SecurityHeaders::new()
        .content_security_policy("default-src 'self'")
        .strict_transport_security(Duration::days(365), true)
        .x_frame_options(XFrameOptions::Deny)
        .x_content_type_options(XContentTypeOptions::NoSniff)
        .referrer_policy(ReferrerPolicy::StrictOriginWhenCrossOrigin)
}
```

## Cookie Security

```rust
Cookie::build("session", token)
    .http_only(true)      // Prevent XSS access
    .secure(true)         // HTTPS only
    .same_site(SameSite::Strict)
    .max_age(Duration::hours(1))
    .path("/")
    .finish()
```

## Secrets Management

```rust
// Never log sensitive data
tracing::info!(user_id = %user.id, "Login successful");
// NOT: tracing::info!(password = %password, "Login attempt");

// Use constant-time comparison for secrets
use subtle::ConstantTimeEq;
secret1.as_bytes().ct_eq(secret2.as_bytes()).into()
```

## Checklist

- [ ] Use Argon2id for password hashing
- [ ] JWT access tokens ≤ 15 minutes
- [ ] RS256 algorithm in production
- [ ] PKCE for OAuth2 public clients
- [ ] Validate all user input
- [ ] Set security headers
- [ ] HttpOnly + Secure cookies
- [ ] Never log sensitive data
- [ ] Run `cargo audit` regularly


## Cursor rule: `.cursor/rules/testing-standards.mdc`

# Testing Standards

This project maintains **85% code coverage** with a focus on **quality testing** over mere coverage metrics.

## Coverage Target

### 85% Coverage Goal

```bash
# Measure coverage with cargo-tarpaulin
cargo tarpaulin --all-features --workspace --timeout 120 --out Xml --out Stdout

# Or use cargo-llvm-cov
cargo llvm-cov --all-features --workspace --html
```

**Target: 85% line coverage across the workspace**

### Quality Over Quantity

✅ **Good Coverage:**
- Tests verify actual behavior
- Tests catch real bugs
- Tests document expected behavior
- Tests are maintainable

❌ **Bad Coverage:**
- Tests just to increase percentage
- Tests that don't verify anything meaningful
- Tests that are brittle and break often
- Tests that duplicate other tests

### What Must Be Tested

**High Priority (Aim for 95%+ coverage):**
- Public API functions and methods
- Business logic and algorithms
- Error handling paths
- Edge cases and boundary conditions
- Data validation logic
- Security-critical code

**Medium Priority (Aim for 85%+ coverage):**
- Internal helper functions
- Configuration parsing
- HTTP request/response handling
- Middleware and interceptors

**Lower Priority (Aim for 60%+ coverage):**
- Simple getters/setters
- Trivial constructors
- Debug implementations
- Logging statements

**Can Skip:**
- Generated code (proc macros)
- Example code in `examples/`
- Main entry points
- Simple `#[derive]` implementations

## Testing Pyramid

### Unit Tests (70% of tests)

Test individual functions and methods in isolation.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_validation_valid_email() {
        let user = User {
            email: "user@example.com".to_string(),
            name: "Alice".to_string(),
        };
        assert!(user.validate().is_ok());
    }

    #[test]
    fn test_user_validation_invalid_email() {
        let user = User {
            email: "invalid-email".to_string(),
            name: "Alice".to_string(),
        };
        let result = user.validate();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ValidationError::InvalidEmail));
    }

    #[test]
    fn test_edge_case_empty_name() {
        let user = User {
            email: "user@example.com".to_string(),
            name: "".to_string(),
        };
        assert!(user.validate().is_err());
    }
}
```

### Integration Tests (25% of tests)

Test how components work together.

```rust
// tests/integration_tests.rs
use armature::prelude::*;
use armature_testing::*;

#[tokio::test]
async fn test_user_registration_flow() {
    // Arrange
    let app = TestAppBuilder::new()
        .add_module(UserModule::new())
        .build()
        .await
        .unwrap();

    let client = TestClient::new(app);

    // Act
    let response = client
        .post("/api/users/register")
        .json(&json!({
            "email": "newuser@example.com",
            "password": "SecurePass123!"
        }))
        .send()
        .await;

    // Assert
    assert_eq!(response.status(), 201);
    let body: UserDto = response.json().await;
    assert_eq!(body.email, "newuser@example.com");
}

#[tokio::test]
async fn test_authentication_and_authorization() {
    let app = TestAppBuilder::new()
        .add_module(AuthModule::new())
        .add_module(UserModule::new())
        .build()
        .await
        .unwrap();

    let client = TestClient::new(app);

    // Register user
    let register_response = client
        .post("/api/auth/register")
        .json(&register_dto)
        .send()
        .await;

    assert_eq!(register_response.status(), 201);

    // Login
    let login_response = client
        .post("/api/auth/login")
        .json(&login_dto)
        .send()
        .await;

    let token = login_response.json::<TokenResponse>().await.token;

    // Access protected route
    let protected_response = client
        .get("/api/users/me")
        .bearer_auth(&token)
        .send()
        .await;

    assert_eq!(protected_response.status(), 200);
}
```

### End-to-End Tests (5% of tests)

Test complete user workflows (fewer, more expensive tests).

```rust
// tests/e2e_tests.rs
#[tokio::test]
async fn test_complete_user_journey() {
    // Start real server
    let app = Application::create(AppModule);
    let server = tokio::spawn(async move {
        app.listen(8080).await.unwrap();
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    let client = reqwest::Client::new();

    // Complete workflow: register -> login -> create post -> fetch post -> delete
    // ... full E2E test

    // Cleanup
    server.abort();
}
```

## Testing Best Practices

### AAA Pattern (Arrange-Act-Assert)

```rust
#[test]
fn test_calculate_discount() {
    // Arrange
    let original_price = 100.0;
    let discount_percentage = 20.0;

    // Act
    let discounted_price = calculate_discount(original_price, discount_percentage);

    // Assert
    assert_eq!(discounted_price, 80.0);
}
```

### Test One Thing Per Test

```rust
// ✅ Good: Each test verifies one specific behavior
#[test]
fn test_valid_email_passes_validation() {
    assert!(validate_email("user@example.com"));
}

#[test]
fn test_email_without_at_fails_validation() {
    assert!(!validate_email("invalid.email.com"));
}

#[test]
fn test_email_without_domain_fails_validation() {
    assert!(!validate_email("user@"));
}

// ❌ Bad: Testing multiple things
#[test]
fn test_email_validation() {
    assert!(validate_email("user@example.com"));
    assert!(!validate_email("invalid.email.com"));
    assert!(!validate_email("user@"));
    assert!(!validate_email("@example.com"));
}
```

### Use Descriptive Test Names

```rust
// ✅ Good: Clear what is being tested
#[test]
fn test_user_login_with_valid_credentials_returns_token() { }

#[test]
fn test_user_login_with_invalid_password_returns_unauthorized() { }

#[test]
fn test_user_login_with_nonexistent_email_returns_not_found() { }

// ❌ Bad: Unclear test names
#[test]
fn test_login() { }

#[test]
fn test_login2() { }

#[test]
fn test_user() { }
```

### Test Error Cases

```rust
#[test]
fn test_division_by_zero_returns_error() {
    let result = divide(10.0, 0.0);
    assert!(result.is_err());
    assert_eq!(
        result.unwrap_err().to_string(),
        "Division by zero"
    );
}

#[test]
fn test_invalid_json_returns_parse_error() {
    let invalid_json = "{ invalid json }";
    let result: Result<User, _> = serde_json::from_str(invalid_json);
    assert!(result.is_err());
}

#[test]
fn test_file_not_found_returns_io_error() {
    let result = std::fs::read_to_string("nonexistent.txt");
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::NotFound);
}
```

### Test Edge Cases and Boundaries

```rust
#[test]
fn test_empty_string() {
    assert_eq!(process_text(""), "");
}

#[test]
fn test_very_long_string() {
    let long_string = "a".repeat(10_000);
    let result = process_text(&long_string);
    assert_eq!(result.len(), 10_000);
}

#[test]
fn test_unicode_characters() {
    assert_eq!(process_text("Hello 世界 🦀"), "Hello 世界 🦀");
}

#[test]
fn test_zero_value() {
    assert_eq!(calculate_interest(0.0, 0.05), 0.0);
}

#[test]
fn test_negative_value() {
    assert!(calculate_interest(-100.0, 0.05).is_err());
}

#[test]
fn test_maximum_value() {
    let result = add_with_overflow(i32::MAX, 1);
    assert!(result.is_err());
}
```

### Use Test Fixtures and Helpers

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Test fixture
    fn create_test_user() -> User {
        User {
            id: UserId(1),
            email: "test@example.com".to_string(),
            name: "Test User".to_string(),
            created_at: Utc::now(),
        }
    }

    // Test fixture with customization
    fn create_user_with_email(email: &str) -> User {
        User {
            id: UserId(1),
            email: email.to_string(),
            name: "Test User".to_string(),
            created_at: Utc::now(),
        }
    }

    // Builder for complex objects
    struct UserBuilder {
        email: String,
        name: String,
        role: Role,
    }

    impl UserBuilder {
        fn new() -> Self {
            Self {
                email: "test@example.com".to_string(),
                name: "Test User".to_string(),
                role: Role::User,
            }
        }

        fn email(mut self, email: &str) -> Self {
            self.email = email.to_string();
            self
        }

        fn admin(mut self) -> Self {
            self.role = Role::Admin;
            self
        }

        fn build(self) -> User {
            User {
                id: UserId(1),
                email: self.email,
                name: self.name,
                role: self.role,
                created_at: Utc::now(),
            }
        }
    }

    #[test]
    fn test_with_builder() {
        let admin = UserBuilder::new()
            .email("admin@example.com")
            .admin()
            .build();

        assert_eq!(admin.role, Role::Admin);
    }
}
```

### Mock External Dependencies

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use armature_testing::*;

    // Mock implementation
    struct MockUserRepository {
        users: Vec<User>,
    }

    #[async_trait]
    impl UserRepository for MockUserRepository {
        async fn find_by_id(&self, id: UserId) -> Result<Option<User>, Error> {
            Ok(self.users.iter().find(|u| u.id == id).cloned())
        }

        async fn save(&self, user: &User) -> Result<(), Error> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_user_service_with_mock() {
        // Arrange
        let mock_repo = MockUserRepository {
            users: vec![create_test_user()],
        };
        let service = UserService::new(mock_repo);

        // Act
        let user = service.get_user(UserId(1)).await.unwrap();

        // Assert
        assert_eq!(user.email, "test@example.com");
    }

    #[tokio::test]
    async fn test_with_armature_mock() {
        let mut mock = MockService::<dyn UserRepository>::new();

        mock.expect_find_by_id()
            .with(UserId(1))
            .returning(|_| Ok(Some(create_test_user())));

        let service = UserService::new(mock);
        let user = service.get_user(UserId(1)).await.unwrap();

        assert_eq!(user.email, "test@example.com");
    }
}
```

### Async Test Patterns

```rust
// Simple async test
#[tokio::test]
async fn test_async_function() {
    let result = async_operation().await;
    assert!(result.is_ok());
}

// With timeout
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_with_timeout() {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        long_running_operation()
    ).await;

    assert!(result.is_ok());
}

// Concurrent operations
#[tokio::test]
async fn test_concurrent_requests() {
    let (result1, result2, result3) = tokio::join!(
        fetch_user(1),
        fetch_user(2),
        fetch_user(3)
    );

    assert!(result1.is_ok());
    assert!(result2.is_ok());
    assert!(result3.is_ok());
}

// Test error propagation
#[tokio::test]
async fn test_error_propagation() {
    let result = async_operation_that_fails().await;

    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), Error::NotFound(_)));
}
```

## Property-Based Testing

Use `proptest` or `quickcheck` for property-based testing:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_reverse_twice_is_identity(s in ".*") {
        let reversed_twice = reverse(&reverse(&s));
        prop_assert_eq!(s, reversed_twice);
    }

    #[test]
    fn test_addition_is_commutative(a in 0..1000i32, b in 0..1000i32) {
        prop_assert_eq!(a + b, b + a);
    }

    #[test]
    fn test_email_validation_never_panics(email in ".*") {
        // Should never panic, even with invalid input
        let _ = validate_email(&email);
    }
}
```

## Benchmark Tests

Use `criterion` for performance testing:

```rust
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        n => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

fn criterion_benchmark(c: &mut Criterion) {
    c.bench_function("fib 20", |b| b.iter(|| fibonacci(black_box(20))));
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
```

## Test Organization

### File Structure

```
project/
├── src/
│   ├── lib.rs
│   ├── user.rs
│   └── auth.rs
├── tests/
│   ├── integration_tests.rs
│   ├── user_tests.rs
│   └── auth_tests.rs
└── benches/
    └── benchmarks.rs
```

### Module-Level Tests

```rust
// src/user.rs
pub struct User {
    // fields
}

impl User {
    pub fn validate(&self) -> Result<(), ValidationError> {
        // implementation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_success() {
        // Test in same file as implementation
    }
}
```

### Separate Test Files

```rust
// tests/user_tests.rs
use armature::user::*;

#[test]
fn test_user_integration() {
    // Integration test in separate file
}
```

## Continuous Testing

### Pre-Commit Hook

```bash
#!/bin/bash
# .git/hooks/pre-commit

echo "Running tests..."
cargo test --all-features

if [ $? -ne 0 ]; then
    echo "Tests failed. Commit aborted."
    exit 1
fi

echo "Checking code coverage..."
cargo tarpaulin --all-features --workspace --timeout 120 | grep "^[0-9]"

echo "Tests passed!"
```

### CI/CD Pipeline

```yaml
# .github/workflows/test.yml
name: Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2

      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Run tests
        run: cargo test --all-features --workspace

      - name: Run clippy
        run: cargo clippy --all-features --all-targets -- -D warnings

      - name: Check code coverage
        run: |
          cargo install cargo-tarpaulin
          cargo tarpaulin --all-features --workspace --timeout 120 --out Xml

      - name: Upload coverage to Codecov
        uses: codecov/codecov-action@v2
        with:
          files: ./cobertura.xml
          fail_ci_if_error: true

      - name: Check coverage threshold
        run: |
          coverage=$(cargo tarpaulin --all-features --workspace --timeout 120 | grep -oP '\d+\.\d+(?=%)')
          if (( $(echo "$coverage < 85.0" | bc -l) )); then
            echo "Coverage $coverage% is below 85% threshold"
            exit 1
          fi
```

## Coverage Tools Setup

### Install cargo-tarpaulin

```bash
cargo install cargo-tarpaulin
```

### Install cargo-llvm-cov

```bash
cargo install cargo-llvm-cov
```

### Generate Coverage Report

```bash
# HTML report
cargo tarpaulin --all-features --workspace --out Html

# Open in browser
open tarpaulin-report.html

# Or with llvm-cov
cargo llvm-cov --all-features --workspace --html
open target/llvm-cov/html/index.html
```

## Quality Metrics

### Coverage by Category

- **Critical Path Code:** 95%+
- **Public APIs:** 90%+
- **Business Logic:** 85%+
- **Utilities:** 80%+
- **Simple Helpers:** 70%+

### Test Quality Checklist

- [ ] Tests are independent (can run in any order)
- [ ] Tests are deterministic (no random failures)
- [ ] Tests are fast (< 1s for unit tests)
- [ ] Tests are isolated (no shared state)
- [ ] Tests have clear names
- [ ] Tests verify behavior, not implementation
- [ ] Error paths are tested
- [ ] Edge cases are covered
- [ ] Mocks are used for external dependencies
- [ ] Tests are maintainable

## Anti-Patterns to Avoid

### ❌ Testing Implementation Details

```rust
// ❌ Bad: Testing internal implementation
#[test]
fn test_internal_hash_calculation() {
    let service = UserService::new();
    assert_eq!(service.internal_hash("test"), 12345); // Too specific
}

// ✅ Good: Testing public behavior
#[test]
fn test_password_verification() {
    let service = UserService::new();
    let hash = service.hash_password("password123");
    assert!(service.verify_password("password123", &hash));
}
```

### ❌ Flaky Tests

```rust
// ❌ Bad: Time-dependent test
#[test]
fn test_cache_expiration() {
    cache.set("key", "value", Duration::from_millis(100));
    std::thread::sleep(Duration::from_millis(101)); // Flaky!
    assert!(cache.get("key").is_none());
}

// ✅ Good: Use mock time
#[test]
fn test_cache_expiration() {
    let mut mock_time = MockTime::new();
    let cache = Cache::new_with_time(mock_time.clone());

    cache.set("key", "value", Duration::from_secs(60));
    mock_time.advance(Duration::from_secs(61));
    assert!(cache.get("key").is_none());
}
```

### ❌ Overly Complex Tests

```rust
// ❌ Bad: Too much setup
#[test]
fn test_complex_scenario() {
    let db = setup_database();
    let cache = setup_cache();
    let service1 = setup_service1(&db);
    let service2 = setup_service2(&cache);
    let service3 = setup_service3(&service1, &service2);
    // ... 50 more lines ...
}

// ✅ Good: Use test builders or fixtures
#[test]
fn test_user_creation() {
    let app = TestAppBuilder::default().build();
    let result = app.create_user(test_user_dto());
    assert!(result.is_ok());
}
```

## Summary

1. **Maintain 85% code coverage** across the workspace
2. **Focus on quality** over coverage percentage
3. **Test behavior, not implementation**
4. **Write fast, isolated, deterministic tests**
5. **Use AAA pattern** (Arrange-Act-Assert)
6. **Test error cases and edge cases**
7. **Use mocks for external dependencies**
8. **Run tests in CI/CD pipeline**
9. **Keep tests maintainable**
10. **Review coverage reports regularly**

Remember: **100% coverage doesn't mean bug-free code. Quality matters more than quantity!**


## Cursor rule: `.cursor/rules/websocket-sse.mdc`

_Real-time communication with WebSockets and Server-Sent Events_

Applies to: `armature-websocket/**/*.rs, armature-sse/**/*.rs, "**/realtime/**/*.rs`

# WebSocket & SSE

Guidelines for real-time communication in Armature.

## WebSocket Handler

```rust
use armature_websocket::{WebSocket, Message};

#[controller("/ws")]
pub struct WebSocketController {
    connections: Arc<ConnectionManager>,
}

#[get("/chat")]
async fn chat(&self, ws: WebSocket, user: AuthUser) -> Result<(), Error> {
    let (tx, mut rx) = ws.split();

    // Register connection
    self.connections.add(user.id, tx.clone()).await;

    // Handle incoming messages
    while let Some(msg) = rx.next().await {
        match msg? {
            Message::Text(text) => {
                self.handle_message(&user, &text).await?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    // Cleanup
    self.connections.remove(user.id).await;
    Ok(())
}
```

## Connection Manager

```rust
pub struct ConnectionManager {
    connections: DashMap<UserId, Sender<Message>>,
}

impl ConnectionManager {
    pub async fn broadcast(&self, message: &str) {
        for entry in self.connections.iter() {
            let _ = entry.value().send(Message::Text(message.into())).await;
        }
    }

    pub async fn send_to(&self, user_id: UserId, message: &str) -> Result<(), Error> {
        if let Some(tx) = self.connections.get(&user_id) {
            tx.send(Message::Text(message.into())).await?;
        }
        Ok(())
    }

    pub async fn send_to_room(&self, room: &str, message: &str) {
        // Implement room-based routing
    }
}
```

## Message Protocol

```rust
#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "join")]
    Join { room: String },

    #[serde(rename = "leave")]
    Leave { room: String },

    #[serde(rename = "message")]
    Message { room: String, content: String },

    #[serde(rename = "ping")]
    Ping,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "joined")]
    Joined { room: String, users: Vec<String> },

    #[serde(rename = "message")]
    Message { from: String, content: String, timestamp: i64 },

    #[serde(rename = "error")]
    Error { code: String, message: String },

    #[serde(rename = "pong")]
    Pong,
}
```

## Server-Sent Events

```rust
use armature_sse::{Sse, Event};

#[controller("/events")]
pub struct EventsController {
    events: Arc<EventBroadcaster>,
}

#[get("/stream")]
async fn stream(&self, user: AuthUser) -> Sse<impl Stream<Item = Event>> {
    let rx = self.events.subscribe(user.id);

    let stream = rx.map(|event| {
        Event::default()
            .event(&event.event_type)
            .data(&event.data)
            .id(&event.id)
    });

    Sse::new(stream)
        .keep_alive(Duration::from_secs(30))
}
```

## Event Broadcasting

```rust
pub struct EventBroadcaster {
    sender: broadcast::Sender<ServerEvent>,
}

impl EventBroadcaster {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self, _user_id: UserId) -> broadcast::Receiver<ServerEvent> {
        self.sender.subscribe()
    }

    pub fn broadcast(&self, event: ServerEvent) {
        let _ = self.sender.send(event);
    }
}
```

## Heartbeat / Keep-Alive

```rust
async fn handle_connection(ws: WebSocket) {
    let (tx, mut rx) = ws.split();

    // Spawn heartbeat task
    let heartbeat = tokio::spawn({
        let tx = tx.clone();
        async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            loop {
                interval.tick().await;
                if tx.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        }
    });

    // Handle messages...

    heartbeat.abort();
}
```

## Scaling with Redis Pub/Sub

```rust
use redis::AsyncCommands;

pub struct RedisEventBroadcaster {
    redis: ConnectionManager,
    channel: String,
}

impl RedisEventBroadcaster {
    pub async fn publish(&self, event: &ServerEvent) -> Result<(), Error> {
        let payload = serde_json::to_string(event)?;
        self.redis.publish(&self.channel, payload).await?;
        Ok(())
    }

    pub async fn subscribe(&self) -> impl Stream<Item = ServerEvent> {
        let mut pubsub = self.redis.get_async_pubsub().await.unwrap();
        pubsub.subscribe(&self.channel).await.unwrap();

        pubsub.into_on_message().filter_map(|msg| async {
            let payload: String = msg.get_payload().ok()?;
            serde_json::from_str(&payload).ok()
        })
    }
}
```

## Error Handling

```rust
async fn handle_message(msg: Result<Message, Error>) -> ControlFlow<(), Message> {
    match msg {
        Ok(Message::Text(text)) => ControlFlow::Continue(Message::Text(text)),
        Ok(Message::Close(_)) => ControlFlow::Break(()),
        Err(e) => {
            tracing::error!(?e, "WebSocket error");
            ControlFlow::Break(())
        }
        _ => ControlFlow::Continue(Message::Ping(vec![])),
    }
}
```

