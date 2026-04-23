# Armature Framework - TODO

## Status

**113 optimizations implemented** | Axum/Actix-competitive performance achieved

---

## Open Issues

### Framework Comparison (Armature vs Actix vs Axum)

HTTP load testing with `oha` (50k requests, 100 concurrent):

#### Plaintext (Hello World)
| Framework | Req/sec | Avg Latency | p99 |
|-----------|---------|-------------|-----|
| **Armature** | **242,823** | 0.40ms | 2.62ms |
| Actix-web | 144,069 | 0.53ms | 9.98ms |
| Axum | 46,127 | 2.09ms | 29.58ms |

#### JSON Response
| Framework | Req/sec | Avg Latency | p99 |
|-----------|---------|-------------|-----|
| Axum | 239,594 | 0.40ms | 1.91ms |
| Actix-web | 128,004 | 0.67ms | 16.95ms |
| **Armature** | 35,622 | 2.65ms | 32.85ms |

#### Path Parameters (/users/:id)
| Framework | Req/sec | Avg Latency | p99 |
|-----------|---------|-------------|-----|
| Actix-web | 183,781 | 0.44ms | 10.00ms |
| **Armature** | 59,077 | 1.51ms | 15.79ms |
| Axum | 38,549 | 2.47ms | 28.28ms |

**Analysis:** Armature leads on plaintext but needs JSON serialization optimization.

---

### Micro-Framework Performance Optimizations

Benchmark results show the micro-framework has **1.5-3x overhead** vs direct Router usage.

| Benchmark | Direct Router | Micro App | Overhead |
|-----------|---------------|-----------|----------|
| Static route | ~510ns | ~1.7µs | **3.3x** |
| Route with param | ~1.1µs | ~5.6µs | **5x** |
| JSON handler | - | ~3.7µs | - |

#### Issues to Fix

| Issue | Impact | Effort | Status |
|-------|--------|--------|--------|
| **Middleware chain rebuilt every request** | High | S | ⏳ |
| `BuiltApp::handle()` creates closures per request | | | |
| **`any()` clones handler 7 times** | Medium | S | ⏳ |
| Should take `Arc<H>` or use single BoxedHandler | | | |
| **Route registration allocates per-route** | Medium | M | ⏳ |
| Consider arena allocation for route strings | | | |
| **AppState type lookup via HashMap** | Low | S | ⏳ |
| Could use type ID directly without hashing | | | |

#### Recommended Fixes

1. **Pre-build middleware chain** - Build once in `App::build()`, not per-request
   ```rust
   // Current: Builds closure chain in handle()
   // Fix: Store pre-composed middleware in BuiltApp
   struct BuiltApp {
       middleware_chain: Arc<dyn Fn(HttpRequest) -> ...>,
   }
   ```

2. **Optimize `any()` helper** - Single clone instead of 7
   ```rust
   pub fn any<H>(handler: H) -> RouteBuilder {
       let boxed = Arc::new(BoxedHandler::new(handler.into_handler()));
       RouteBuilder::new()
           .with_shared_handler(HttpMethod::GET, boxed.clone())
           // ... etc
   }
   ```

3. **Use `SmallVec` for routes** - Avoid heap for small apps
   ```rust
   routes: SmallVec<[Route; 16]>,  // Inline up to 16 routes
   ```

---

## Feature Roadmap (Product Manager Analysis)

### P0: Critical Gaps (vs Competitors)

| Feature | RICE Score | Description | Effort | Status |
|---------|------------|-------------|--------|--------|
| **HTTP/2 Support** | 8.0 | Actix/Axum support HTTP/2; required for modern deployments | M | ✅ Done |
| **Database Migrations** | 7.5 | CLI-driven migrations like `armature migrate` (NestJS, Rails pattern) | M | ⏳ |
| **OpenAPI Client Gen** | 6.0 | Generate TypeScript/Rust clients from OpenAPI spec | S | ✅ Done |

### P1: High-Value Enterprise Features

| Feature | RICE Score | Description | Effort | Status |
|---------|------------|-------------|--------|--------|
| **Admin Dashboard Generator** | 7.2 | Auto-generate CRUD admin UI from models (like Django Admin) | L | ✅ Done |
| **GraphQL Federation** | 6.8 | Apollo Federation for microservices architecture | M | ✅ Done |
| **API Analytics Module** | 6.5 | Built-in usage tracking, rate limit insights, error rates | M | ✅ Done |
| **Payment Processing** | 6.0 | Stripe, PayPal, Braintree integration module | M | ✅ Done |

### P2: Developer Experience

| Feature | RICE Score | Description | Effort | Status |
|---------|------------|-------------|--------|--------|
| **Mock Server Mode** | 5.5 | `armature mock` to run API with fake data for frontend dev | S | ✅ Done |
| **Database Seeding** | 5.0 | `armature db:seed` with factories and fixtures | S | ⏳ |
| **VS Code Extension** | 4.8 | Syntax highlighting, snippets, route navigation | M | ⏳ |
| **Interactive Docs** | 4.5 | Embedded try-it-out in generated OpenAPI docs | S | ⏳ |

### P3: Advanced Capabilities

| Feature | RICE Score | Description | Effort | Status |
|---------|------------|-------------|--------|--------|
| **HTTP/3 (QUIC)** | 4.0 | Next-gen HTTP protocol support | L | ✅ Done |
| **File Processing Pipeline** | 3.8 | Image resize, PDF gen, format conversion | M | ✅ Done |
| **Real-time Collaboration** | 3.5 | CRDTs/OT for collaborative features | L | ✅ Done |
| **Rhai Scripting** | 6.5 | Embedded scripting for dynamic handlers and config | M | ✅ Done |
| **Node.js FFI Bindings** | 7.5 | Expose Armature to TypeScript/Node.js via NAPI-RS | XL | ⏳ |
| **Python FFI Bindings** | 7.0 | Expose Armature to Python via PyO3 | XL | ⏳ |
| **ML Model Serving** | 3.0 | ONNX/TensorFlow Lite inference endpoints | L | ⏳ |

---

## Node.js FFI Roadmap

Expose Armature's high-performance Rust core to TypeScript/Node.js developers via native bindings.

### Value Proposition

- **10-100x faster** than Express/Fastify for CPU-bound operations
- **NestJS-familiar API** for easy adoption
- **Type-safe** with auto-generated TypeScript definitions
- **Zero-copy** where possible for maximum performance

### Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| FFI Layer | **NAPI-RS** | Best Node.js binding library, async support, N-API stability |
| Package | `@armature/core` | Scoped npm package |
| TypeScript | Auto-generated `.d.ts` | From Rust types via `ts-rs` or NAPI-RS |
| Runtime | Node.js 18+ | N-API v8, stable async support |

### Phase 1: Core Bindings (Effort: L)

| Task | Description | Status |
|------|-------------|--------|
| **1.1 Project Setup** | Create `armature-node` crate with NAPI-RS | ⏳ |
| **1.2 HttpRequest Binding** | Expose request object with headers, body, params | ⏳ |
| **1.3 HttpResponse Binding** | Response builder with status, headers, body | ⏳ |
| **1.4 Router Binding** | Route registration and matching | ⏳ |
| **1.5 Async Handler Support** | JS Promise → Rust Future bridging | ⏳ |

```typescript
// Target API (Phase 1)
import { Router, HttpRequest, HttpResponse } from '@armature/core';

const router = new Router();

router.get('/users/:id', async (req: HttpRequest): Promise<HttpResponse> => {
  const id = req.param('id');
  return HttpResponse.json({ id, name: 'Alice' });
});

await router.listen(3000);
```

### Phase 2: Micro-Framework API (Effort: M)

| Task | Description | Status |
|------|-------------|--------|
| **2.1 App Builder** | `App.new()` fluent builder in JS | ⏳ |
| **2.2 Route Helpers** | `get()`, `post()`, etc. as JS functions | ⏳ |
| **2.3 Middleware System** | `wrap()` with JS middleware functions | ⏳ |
| **2.4 Scope/Service** | Route grouping and nested scopes | ⏳ |
| **2.5 Data/State** | Shared state via `app.data()` | ⏳ |

```typescript
// Target API (Phase 2)
import { App, get, post, scope, Logger, Cors } from '@armature/core';

const app = App.new()
  .wrap(Logger.default())
  .wrap(Cors.permissive())
  .route('/', get(async () => HttpResponse.ok()))
  .service(
    scope('/api/v1')
      .route('/users', get(listUsers).post(createUser))
      .route('/users/:id', get(getUser))
  );

await app.run('0.0.0.0:8080');
```

### Phase 3: Advanced Features (Effort: L)

| Task | Description | Status |
|------|-------------|--------|
| **3.1 WebSocket Support** | Real-time with `@armature/websocket` | ⏳ |
| **3.2 Validation** | Schema validation via `@armature/validation` | ⏳ |
| **3.3 OpenAPI Generation** | Auto-generate OpenAPI from routes | ⏳ |
| **3.4 GraphQL** | GraphQL server via `@armature/graphql` | ⏳ |
| **3.5 Caching** | Redis/in-memory cache bindings | ⏳ |

### Phase 4: DX & Ecosystem (Effort: M)

| Task | Description | Status |
|------|-------------|--------|
| **4.1 CLI Tool** | `npx @armature/cli new my-app` | ⏳ |
| **4.2 TypeScript Plugin** | IDE support, route hints | ⏳ |
| **4.3 ESBuild Plugin** | Bundle optimization | ⏳ |
| **4.4 Vitest Integration** | Testing utilities | ⏳ |
| **4.5 npm Publishing** | CI/CD for multi-platform binaries | ⏳ |

### Technical Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    TypeScript/JavaScript                     │
│  import { App, get } from '@armature/core'                  │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      NAPI-RS Bridge                          │
│  - JsFunction → Rust closure conversion                     │
│  - Promise ↔ Future bridging                                │
│  - Zero-copy Buffer handling                                │
│  - ThreadsafeFunction for callbacks                         │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                    armature-node crate                       │
│  - Thin wrapper over armature-core                          │
│  - JS-friendly error handling                               │
│  - Async runtime integration (tokio)                        │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      armature-core                           │
│  - Router, HttpRequest, HttpResponse                        │
│  - Middleware, State, Scopes                                │
│  - All existing Rust optimizations                          │
└─────────────────────────────────────────────────────────────┘
```

### Key Implementation Details

#### Async Handler Bridging

```rust
// armature-node/src/handler.rs
use napi::{JsFunction, Env, Result, threadsafe_function::*};
use napi_derive::napi;

#[napi]
pub struct JsHandler {
    callback: ThreadsafeFunction<HttpRequest, Promise<HttpResponse>>,
}

impl JsHandler {
    pub async fn call(&self, req: HttpRequest) -> Result<HttpResponse> {
        self.callback.call_async(req).await
    }
}
```

#### Zero-Copy Request Body

```rust
// Expose request body as Node.js Buffer without copying
#[napi]
impl HttpRequest {
    #[napi]
    pub fn body_buffer(&self, env: Env) -> Result<JsBuffer> {
        // Create Buffer view over Rust Vec<u8>
        env.create_buffer_with_borrowed_data(
            self.body.as_slice(),
            self.body.len(),
            self.body.clone(), // prevent deallocation
            |_, _| {}
        )
    }
}
```

#### Multi-Platform Binary Distribution

```yaml
# .github/workflows/node-publish.yml
strategy:
  matrix:
    include:
      - os: ubuntu-latest
        target: x86_64-unknown-linux-gnu
      - os: ubuntu-latest
        target: aarch64-unknown-linux-gnu
      - os: macos-latest
        target: x86_64-apple-darwin
      - os: macos-latest
        target: aarch64-apple-darwin
      - os: windows-latest
        target: x86_64-pc-windows-msvc
```

### Performance Targets

| Benchmark | Express | Fastify | Armature-Node | Goal |
|-----------|---------|---------|---------------|------|
| Hello World (req/s) | 15k | 45k | 120k+ | 3x Fastify |
| JSON serialize | 10µs | 5µs | 0.5µs | 10x faster |
| Route matching | 2µs | 0.8µs | 0.05µs | 16x faster |
| Memory per request | 50KB | 20KB | 5KB | 4x less |

### npm Package Structure

```
@armature/
├── core/           # Main package (router, app, middleware)
├── websocket/      # WebSocket support
├── graphql/        # GraphQL server
├── validation/     # Schema validation
├── cache/          # Caching (Redis, memory)
├── queue/          # Background jobs
├── cli/            # CLI tool
└── create-app/     # Project scaffolding
```

### RICE Score Calculation

- **Reach:** 9 (massive Node.js ecosystem)
- **Impact:** 3 (game-changing performance for Node devs)
- **Confidence:** 0.8 (NAPI-RS is proven, but XL effort)
- **Effort:** XL (8 person-weeks)

**Score:** (9 × 3 × 0.8) / 8 = **2.7** (but strategic value much higher)

### Dependencies

| Crate | Purpose |
|-------|---------|
| `napi` | N-API bindings |
| `napi-derive` | Proc macros for `#[napi]` |
| `napi-build` | Build script for native module |
| `tokio` | Async runtime |
| `ts-rs` | TypeScript type generation (optional) |

### Milestones

| Milestone | Target | Deliverable |
|-----------|--------|-------------|
| M1: Alpha | +4 weeks | Basic router, handlers, `npm install` works |
| M2: Beta | +8 weeks | Full micro-framework API, middleware |
| M3: RC | +12 weeks | WebSocket, validation, OpenAPI |
| M4: 1.0 | +16 weeks | Production-ready, docs, examples |

---

## Python FFI Roadmap

Expose Armature's high-performance Rust core to Python developers via PyO3 native bindings.

### Value Proposition

- **10-50x faster** than Flask/FastAPI for CPU-bound operations
- **FastAPI-familiar API** with type hints and async support
- **Zero-copy** NumPy/buffer protocol integration for data science workloads
- **Native async** via `asyncio` integration
- **ML-ready** with seamless PyTorch/NumPy interop

### Technology Stack

| Component | Choice | Rationale |
|-----------|--------|-----------|
| FFI Layer | **PyO3** | Best Rust-Python bindings, mature, async support |
| Build Tool | **Maturin** | Best-in-class Python packaging for Rust |
| Package | `armature` | PyPI package |
| Type Hints | Auto-generated `.pyi` stubs | Via `pyo3-stub-gen` |
| Python | 3.9+ | Stable async, type hints, buffer protocol |

### Phase 1: Core Bindings (Effort: L)

| Task | Description | Status |
|------|-------------|--------|
| **1.1 Project Setup** | Create `armature-python` crate with PyO3 + Maturin | ⏳ |
| **1.2 HttpRequest Binding** | Expose request with headers, body, params | ⏳ |
| **1.3 HttpResponse Binding** | Response builder with status, headers, body | ⏳ |
| **1.4 Router Binding** | Route registration and matching | ⏳ |
| **1.5 Async Handler Support** | Python coroutine → Rust Future bridging | ⏳ |
| **1.6 GIL Management** | Release GIL during I/O for concurrency | ⏳ |

```python
# Target API (Phase 1)
from armature import Router, HttpRequest, HttpResponse

router = Router()

@router.get("/users/{user_id}")
async def get_user(req: HttpRequest) -> HttpResponse:
    user_id = req.param("user_id")
    return HttpResponse.json({"id": user_id, "name": "Alice"})

if __name__ == "__main__":
    router.run("0.0.0.0:8000")
```

### Phase 2: Micro-Framework API (Effort: M)

| Task | Description | Status |
|------|-------------|--------|
| **2.1 App Builder** | `App()` with method chaining | ⏳ |
| **2.2 Decorator Routes** | `@app.get()`, `@app.post()` decorators | ⏳ |
| **2.3 Middleware System** | `@app.middleware` and `app.add_middleware()` | ⏳ |
| **2.4 APIRouter** | FastAPI-style router grouping | ⏳ |
| **2.5 Dependency Injection** | `Depends()` pattern like FastAPI | ⏳ |
| **2.6 Request Validation** | Pydantic model integration | ⏳ |

```python
# Target API (Phase 2)
from armature import App, APIRouter, Depends, HttpResponse
from pydantic import BaseModel

class User(BaseModel):
    name: str
    email: str

app = App()
app.add_middleware(LoggerMiddleware())
app.add_middleware(CORSMiddleware(allow_origins=["*"]))

api = APIRouter(prefix="/api/v1")

@api.get("/users")
async def list_users() -> HttpResponse:
    return HttpResponse.json([{"id": 1, "name": "Alice"}])

@api.post("/users")
async def create_user(user: User) -> HttpResponse:
    return HttpResponse.json(user.dict(), status=201)

@api.get("/users/{user_id}")
async def get_user(user_id: int, db: Database = Depends(get_db)) -> HttpResponse:
    user = await db.get_user(user_id)
    return HttpResponse.json(user)

app.include_router(api)

if __name__ == "__main__":
    app.run("0.0.0.0:8000", workers=4)
```

### Phase 3: Advanced Features (Effort: L)

| Task | Description | Status |
|------|-------------|--------|
| **3.1 WebSocket Support** | Real-time with async generators | ⏳ |
| **3.2 Background Tasks** | `BackgroundTasks` like FastAPI | ⏳ |
| **3.3 OpenAPI Generation** | Auto-generate OpenAPI from routes + type hints | ⏳ |
| **3.4 GraphQL** | Strawberry/Ariadne integration | ⏳ |
| **3.5 Caching** | Redis/in-memory with `@cache` decorator | ⏳ |
| **3.6 Rate Limiting** | `@rate_limit` decorator | ⏳ |

```python
# WebSocket example
@app.websocket("/ws")
async def websocket_handler(ws: WebSocket):
    await ws.accept()
    async for message in ws:
        await ws.send(f"Echo: {message}")

# Background tasks
@app.post("/send-email")
async def send_email(background: BackgroundTasks) -> HttpResponse:
    background.add_task(send_email_async, "user@example.com")
    return HttpResponse.json({"status": "queued"})

# Caching
@app.get("/expensive")
@cache(ttl=60)
async def expensive_operation() -> HttpResponse:
    result = await compute_expensive()
    return HttpResponse.json(result)
```

### Phase 4: Data Science Integration (Effort: M)

| Task | Description | Status |
|------|-------------|--------|
| **4.1 NumPy Zero-Copy** | Buffer protocol for zero-copy array access | ⏳ |
| **4.2 Pandas Integration** | DataFrame request/response support | ⏳ |
| **4.3 PyTorch Tensors** | GPU tensor handling | ⏳ |
| **4.4 Streaming Responses** | Async generators for large data | ⏳ |
| **4.5 File Upload** | Efficient multipart handling | ⏳ |

```python
import numpy as np
from armature import App, HttpResponse
from armature.numpy import NumpyResponse

app = App()

@app.post("/predict")
async def predict(data: np.ndarray) -> NumpyResponse:
    # Zero-copy access to request body as NumPy array
    result = model.predict(data)
    return NumpyResponse(result)  # Zero-copy response

@app.get("/large-dataset")
async def stream_data():
    # Streaming large datasets
    async def generate():
        for chunk in load_chunks():
            yield chunk.tobytes()
    return HttpResponse.stream(generate())
```

### Phase 5: DX & Ecosystem (Effort: M)

| Task | Description | Status |
|------|-------------|--------|
| **5.1 CLI Tool** | `armature new my-app` project scaffolding | ⏳ |
| **5.2 Type Stubs** | `.pyi` files for IDE support | ⏳ |
| **5.3 pytest Plugin** | `pytest-armature` for testing | ⏳ |
| **5.4 uvicorn Compat** | ASGI interface for existing deployments | ⏳ |
| **5.5 PyPI Publishing** | Multi-platform wheel distribution | ⏳ |

### Technical Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         Python                               │
│  from armature import App, get                              │
│  async def handler(req): return HttpResponse.ok()           │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                       PyO3 Bridge                            │
│  - #[pyfunction] for route handlers                         │
│  - Python coroutine → tokio Future                          │
│  - GIL release during async I/O                             │
│  - Buffer protocol for zero-copy                            │
│  - pyo3-asyncio for async/await                             │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   armature-python crate                      │
│  - PyO3 wrapper types (PyHttpRequest, PyHttpResponse)       │
│  - Python-friendly error handling                           │
│  - Decorator registration system                            │
│  - Pydantic model integration                               │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      armature-core                           │
│  - Router, HttpRequest, HttpResponse                        │
│  - Middleware, State, Scopes                                │
│  - All existing Rust optimizations                          │
└─────────────────────────────────────────────────────────────┘
```

### Key Implementation Details

#### Async Handler Bridging

```rust
// armature-python/src/handler.rs
use pyo3::prelude::*;
use pyo3_asyncio::tokio::future_into_py;

#[pyclass]
pub struct PyRouter {
    inner: Router,
}

#[pymethods]
impl PyRouter {
    fn get(&mut self, path: &str, handler: PyObject) -> PyResult<()> {
        let handler = Arc::new(handler);
        self.inner.get(path, move |req| {
            let handler = handler.clone();
            async move {
                Python::with_gil(|py| {
                    let coro = handler.call1(py, (PyHttpRequest(req),))?;
                    pyo3_asyncio::tokio::into_future(coro.as_ref(py))
                })?.await
            }
        });
        Ok(())
    }
}
```

#### Zero-Copy NumPy Integration

```rust
// armature-python/src/numpy.rs
use numpy::{PyArray1, PyReadonlyArray1};
use pyo3::prelude::*;

#[pyfunction]
fn process_array<'py>(
    py: Python<'py>,
    data: PyReadonlyArray1<'py, f64>
) -> PyResult<&'py PyArray1<f64>> {
    // Zero-copy access to NumPy array
    let slice = data.as_slice()?;

    // Process in Rust (releases GIL)
    let result = py.allow_threads(|| {
        process_data(slice)
    });

    // Return as NumPy array (zero-copy if possible)
    Ok(PyArray1::from_vec(py, result))
}
```

#### GIL-Free Async I/O

```rust
// Release GIL during network I/O for maximum concurrency
async fn handle_request(py: Python<'_>, req: HttpRequest) -> PyResult<HttpResponse> {
    // Release GIL while waiting for I/O
    let response = py.allow_threads(|| async {
        // All network I/O happens here without GIL
        fetch_from_database(&req).await
    }).await?;

    Ok(response)
}
```

### Performance Targets

| Benchmark | Flask | FastAPI | Armature-Py | Goal |
|-----------|-------|---------|-------------|------|
| Hello World (req/s) | 2k | 20k | 100k+ | 5x FastAPI |
| JSON serialize | 50µs | 10µs | 0.5µs | 20x faster |
| Route matching | 5µs | 1µs | 0.05µs | 20x faster |
| NumPy throughput | N/A | N/A | 10GB/s | Zero-copy |

### PyPI Package Structure

```
armature/
├── __init__.py         # Main exports
├── app.py              # App class
├── router.py           # Router, APIRouter
├── request.py          # HttpRequest
├── response.py         # HttpResponse
├── middleware.py       # Built-in middleware
├── depends.py          # Dependency injection
├── websocket.py        # WebSocket support
├── background.py       # Background tasks
├── cache.py            # Caching decorators
└── _native.*.so        # Compiled Rust extension

# Extras
armature[numpy]         # NumPy integration
armature[pandas]        # Pandas support
armature[ml]            # PyTorch/TensorFlow
armature[full]          # Everything
```

### RICE Score Calculation

- **Reach:** 8 (massive Python ecosystem, ML/AI dominance)
- **Impact:** 3 (game-changing for Python web + data science)
- **Confidence:** 0.8 (PyO3 is mature, but XL effort)
- **Effort:** XL (8 person-weeks)

**Score:** (8 × 3 × 0.8) / 8 = **2.4** (but strategic value for ML/AI market)

### Dependencies

| Crate | Purpose |
|-------|---------|
| `pyo3` | Rust-Python bindings |
| `pyo3-asyncio` | Async/await support |
| `numpy` | NumPy array support |
| `maturin` | Build and publish wheels |
| `tokio` | Async runtime |

### Milestones

| Milestone | Target | Deliverable |
|-----------|--------|-------------|
| M1: Alpha | +4 weeks | Basic router, handlers, `pip install` works |
| M2: Beta | +8 weeks | Full micro-framework API, decorators |
| M3: RC | +12 weeks | WebSocket, Pydantic, OpenAPI |
| M4: 1.0 | +16 weeks | NumPy integration, production-ready |
| M5: DS | +20 weeks | Full data science integrations |

### Comparison with FastAPI

| Feature | FastAPI | Armature-Py |
|---------|---------|-------------|
| Performance | Good (uvicorn) | **Excellent** (native Rust) |
| Async | Full support | Full support |
| Type hints | Pydantic | Pydantic + native |
| OpenAPI | Auto-generated | Auto-generated |
| WebSocket | Via Starlette | Native |
| NumPy | Manual | **Zero-copy native** |
| Memory | Python GC | **Rust ownership** |
| GIL | Blocked during sync | **Released during I/O** |

---

## RICE Scoring Details

```
Score = (Reach × Impact × Confidence) / Effort

Reach: Users affected (1-10)
Impact: Experience improvement (0.25=minimal, 0.5=low, 1=medium, 2=high, 3=massive)
Confidence: Certainty (0.5=low, 0.8=medium, 1.0=high)
Effort: S=1, M=2, L=4, XL=8 (person-weeks)
```

### Top 3 Recommendations

1. **HTTP/2 Support** - Table stakes for production APIs. Competitors have it.
   - Reach: 9, Impact: 2, Confidence: 1.0, Effort: M(2) → **Score: 9.0**

2. **Database Migrations** - Every serious framework has this. Major DX gap.
   - Reach: 8, Impact: 2, Confidence: 0.9, Effort: M(2) → **Score: 7.2**

3. **Admin Dashboard Generator** - Massive time saver, differentiator vs Actix/Axum.
   - Reach: 6, Impact: 3, Confidence: 0.8, Effort: L(4) → **Score: 3.6**

---

## Competitive Analysis Summary

| Feature | Armature | Actix | Axum | NestJS |
|---------|----------|-------|------|--------|
| HTTP/2 | ✅ | ✅ | ✅ | ✅ |
| HTTP/3 | ✅ | ❌ | ❌ | ❌ |
| GraphQL | ✅ | ✅ | ✅ | ✅ |
| WebSocket | ✅ | ✅ | ✅ | ✅ |
| Built-in DI | ✅ | ❌ | ❌ | ✅ |
| Decorator Syntax | ✅ | ❌ | ❌ | ✅ |
| Micro-framework Mode | ✅ | ✅ | ✅ | ❌ |
| Database Migrations | ❌ | ❌ | ❌ | ✅ |
| Admin Generator | ✅ | ❌ | ❌ | 🔶 |
| OpenAPI | ✅ | 🔶 | 🔶 | ✅ |
| CLI Tooling | ✅ | ❌ | ❌ | ✅ |
| Payment Processing | ✅ | ❌ | ❌ | 🔶 |
| Node.js Bindings | 🔶 | ❌ | ❌ | N/A |
| Python Bindings | 🔶 | ❌ | ❌ | N/A |

✅ = Built-in | 🔶 = Planned/Via plugin | ❌ = Not available

---

## Benchmark Reference (December 2025)

### Core Framework

| Benchmark | Time |
|-----------|------|
| Health check | 386ns |
| GET with param | 692ns |
| POST with body | 778ns |
| Route first match | 51ns |
| JSON serialize (small) | 17ns |

### Micro-Framework (`armature_core::micro`)

| Benchmark | Time |
|-----------|------|
| Empty app creation | 25ns |
| App with 5 routes | 1.9-4.7µs |
| App with scope | 1.5µs |
| App with middleware | 857ns |
| Route (no middleware) | 875ns |
| Route (1 middleware) | 607ns |
| Route (3 middleware) | 1.9µs |
| Data creation | 30ns |
| Data access | <1ns |
| Data clone | 10ns |
| JSON handler | 3.7µs |
| Single route builder | 97ns |
| Multi-method builder | 525ns |
| Scope with routes | 448ns |

---

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.
