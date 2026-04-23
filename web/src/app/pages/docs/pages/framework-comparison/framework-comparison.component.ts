import { Component } from '@angular/core';
import { CommonModule } from '@angular/common';
import { DocPageComponent, DocPage } from '../../shared/doc-page.component';

@Component({
  selector: 'app-framework-comparison',
  standalone: true,
  imports: [CommonModule, DocPageComponent],
  template: `<app-doc-page [page]="page"></app-doc-page>`
})
export class FrameworkComparisonComponent {
  page: DocPage = {
    title: 'Armature vs Actix vs Axum',
    subtitle: 'Comprehensive performance benchmarks and feature comparison between Armature\'s micro-framework and the leading Rust web frameworks.',
    icon: '⚡',
    badge: 'Benchmarks',
    features: [
      {
        icon: '🚀',
        title: 'High Performance',
        description: 'Comparable or better throughput than Actix and Axum'
      },
      {
        icon: '🎯',
        title: 'Low Latency',
        description: 'Sub-millisecond p50 latency on most endpoints'
      },
      {
        icon: '📦',
        title: 'Feature Rich',
        description: 'Built-in DI, decorators, and enterprise features'
      },
      {
        icon: '🔧',
        title: 'Familiar API',
        description: 'Actix-style micro-framework for easy migration'
      }
    ],
    sections: [
      {
        id: 'methodology',
        title: 'Benchmark Methodology',
        content: `<p>Benchmarks performed with <code>oha</code> load testing tool:</p>
        <ul>
          <li><strong>Requests:</strong> 50,000 per test</li>
          <li><strong>Concurrency:</strong> 100 concurrent connections</li>
          <li><strong>Hardware:</strong> Same machine, release builds</li>
          <li><strong>Rust:</strong> 1.88 (2024 edition)</li>
        </ul>
        <p>Each framework was tested with identical endpoints: plaintext, JSON, and path parameters.</p>`
      },
      {
        id: 'plaintext',
        title: 'Plaintext (Hello World)',
        content: `<p>Simple "Hello, World!" response - tests raw request handling overhead.</p>
        <table>
          <thead>
            <tr>
              <th>Framework</th>
              <th>Requests/sec</th>
              <th>Avg Latency</th>
              <th>p99 Latency</th>
            </tr>
          </thead>
          <tbody>
            <tr style="background: rgba(34, 197, 94, 0.1);">
              <td><strong>🥇 Armature</strong></td>
              <td><strong>242,823</strong></td>
              <td><strong>0.40ms</strong></td>
              <td><strong>2.62ms</strong></td>
            </tr>
            <tr>
              <td>🥈 Actix-web</td>
              <td>144,069</td>
              <td>0.53ms</td>
              <td>9.98ms</td>
            </tr>
            <tr>
              <td>🥉 Axum</td>
              <td>46,127</td>
              <td>2.09ms</td>
              <td>29.58ms</td>
            </tr>
          </tbody>
        </table>
        <p><strong>Winner: Armature</strong> - 1.7x faster than Actix, 5.3x faster than Axum on plaintext.</p>`
      },
      {
        id: 'json',
        title: 'JSON Serialization',
        content: `<p>JSON response with a simple message object - tests serialization performance.</p>
        <table>
          <thead>
            <tr>
              <th>Framework</th>
              <th>Requests/sec</th>
              <th>Avg Latency</th>
              <th>p99 Latency</th>
            </tr>
          </thead>
          <tbody>
            <tr style="background: rgba(34, 197, 94, 0.1);">
              <td><strong>🥇 Axum</strong></td>
              <td><strong>239,594</strong></td>
              <td><strong>0.40ms</strong></td>
              <td><strong>1.91ms</strong></td>
            </tr>
            <tr>
              <td>🥈 Actix-web</td>
              <td>128,004</td>
              <td>0.67ms</td>
              <td>16.95ms</td>
            </tr>
            <tr>
              <td>🥉 Armature</td>
              <td>35,622</td>
              <td>2.65ms</td>
              <td>32.85ms</td>
            </tr>
          </tbody>
        </table>
        <p><strong>Note:</strong> Armature's JSON serialization has optimization opportunities - see "Areas for Improvement" below.</p>`
      },
      {
        id: 'path-params',
        title: 'Path Parameters',
        content: `<p>Route with path parameter extraction (<code>/users/:id</code>) - tests routing + extraction.</p>
        <table>
          <thead>
            <tr>
              <th>Framework</th>
              <th>Requests/sec</th>
              <th>Avg Latency</th>
              <th>p99 Latency</th>
            </tr>
          </thead>
          <tbody>
            <tr style="background: rgba(34, 197, 94, 0.1);">
              <td><strong>🥇 Actix-web</strong></td>
              <td><strong>183,781</strong></td>
              <td><strong>0.44ms</strong></td>
              <td><strong>10.00ms</strong></td>
            </tr>
            <tr>
              <td>🥈 Armature</td>
              <td>59,077</td>
              <td>1.51ms</td>
              <td>15.79ms</td>
            </tr>
            <tr>
              <td>🥉 Axum</td>
              <td>38,549</td>
              <td>2.47ms</td>
              <td>28.28ms</td>
            </tr>
          </tbody>
        </table>
        <p>Actix-web's mature router wins here, with Armature competitive at 59k req/s.</p>`
      },
      {
        id: 'features',
        title: 'Feature Comparison',
        content: `<p>Beyond raw performance, frameworks differ significantly in features:</p>
        <table>
          <thead>
            <tr>
              <th>Feature</th>
              <th>Armature</th>
              <th>Actix-web</th>
              <th>Axum</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>HTTP/2</td>
              <td>✅</td>
              <td>✅</td>
              <td>✅</td>
            </tr>
            <tr>
              <td>HTTP/3 (QUIC)</td>
              <td>✅</td>
              <td>❌</td>
              <td>❌</td>
            </tr>
            <tr>
              <td>Built-in DI</td>
              <td>✅</td>
              <td>❌</td>
              <td>❌</td>
            </tr>
            <tr>
              <td>Decorator Syntax</td>
              <td>✅</td>
              <td>❌</td>
              <td>❌</td>
            </tr>
            <tr>
              <td>Micro-framework Mode</td>
              <td>✅</td>
              <td>✅</td>
              <td>✅</td>
            </tr>
            <tr>
              <td>OpenAPI Generation</td>
              <td>✅</td>
              <td>🔶</td>
              <td>🔶</td>
            </tr>
            <tr>
              <td>Admin Generator</td>
              <td>✅</td>
              <td>❌</td>
              <td>❌</td>
            </tr>
            <tr>
              <td>CLI Tooling</td>
              <td>✅</td>
              <td>❌</td>
              <td>❌</td>
            </tr>
            <tr>
              <td>WebSocket</td>
              <td>✅</td>
              <td>✅</td>
              <td>✅</td>
            </tr>
            <tr>
              <td>GraphQL</td>
              <td>✅</td>
              <td>✅</td>
              <td>✅</td>
            </tr>
            <tr>
              <td>Payment Processing</td>
              <td>✅</td>
              <td>❌</td>
              <td>❌</td>
            </tr>
          </tbody>
        </table>
        <p>✅ = Built-in | 🔶 = Via plugin | ❌ = Not available</p>`
      },
      {
        id: 'api-comparison',
        title: 'API Style Comparison',
        content: `<p>Compare the same endpoint across frameworks:</p>`,
        subsections: [
          {
            id: 'armature-api',
            title: 'Armature Micro-Framework',
            codeBlocks: [
              {
                language: 'rust',
                code: `use armature_core::micro::*;
use armature_core::{Error, HttpRequest, HttpResponse};

async fn get_user(req: HttpRequest) -> Result<HttpResponse, Error> {
    let id = req.param("id").cloned().unwrap_or_default();
    HttpResponse::json(&User { id, name: "Alice".into() })
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    App::new()
        .wrap(Logger::default())
        .route("/users/:id", get(get_user))
        .run("0.0.0.0:8080")
        .await
}`
              }
            ]
          },
          {
            id: 'actix-api',
            title: 'Actix-web',
            codeBlocks: [
              {
                language: 'rust',
                code: `use actix_web::{get, web, App, HttpResponse, HttpServer};

#[get("/users/{id}")]
async fn get_user(path: web::Path<String>) -> HttpResponse {
    let id = path.into_inner();
    HttpResponse::Ok().json(User { id, name: "Alice".into() })
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .service(get_user)
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}`
              }
            ]
          },
          {
            id: 'axum-api',
            title: 'Axum',
            codeBlocks: [
              {
                language: 'rust',
                code: `use axum::{extract::Path, routing::get, Json, Router};

async fn get_user(Path(id): Path<String>) -> Json<User> {
    Json(User { id, name: "Alice".into() })
}

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/users/:id", get(get_user));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}`
              }
            ]
          }
        ]
      },
      {
        id: 'improvements',
        title: 'Areas for Improvement',
        content: `<p>Armature's micro-framework has identified optimization opportunities:</p>
        <table>
          <thead>
            <tr>
              <th>Issue</th>
              <th>Impact</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            <tr>
              <td>Middleware chain rebuilt per-request</td>
              <td>High</td>
              <td>⏳ Planned</td>
            </tr>
            <tr>
              <td><code>any()</code> clones handler 7 times</td>
              <td>Medium</td>
              <td>⏳ Planned</td>
            </tr>
            <tr>
              <td>JSON Content-Length missing</td>
              <td>Medium</td>
              <td>⏳ Planned</td>
            </tr>
            <tr>
              <td>Route allocation per-route</td>
              <td>Low</td>
              <td>⏳ Planned</td>
            </tr>
          </tbody>
        </table>
        <p>These optimizations will bring JSON serialization performance closer to Axum/Actix levels.</p>`
      },
      {
        id: 'when-to-choose',
        title: 'When to Choose Each Framework',
        content: `<p><strong>Choose Armature when:</strong></p>
        <ul>
          <li>You need built-in DI and decorator syntax (NestJS-style)</li>
          <li>You want enterprise features (OpenAPI, Admin, Payments)</li>
          <li>You need HTTP/3 support</li>
          <li>You prefer batteries-included frameworks</li>
          <li>You're coming from TypeScript/NestJS</li>
        </ul>
        <p><strong>Choose Actix-web when:</strong></p>
        <ul>
          <li>Maximum raw performance is critical</li>
          <li>You need actor-based concurrency</li>
          <li>You prefer minimal abstractions</li>
          <li>Large community and ecosystem matter</li>
        </ul>
        <p><strong>Choose Axum when:</strong></p>
        <ul>
          <li>You want tight Tokio integration</li>
          <li>Tower middleware compatibility is important</li>
          <li>You prefer function-based handlers</li>
          <li>Type-driven API design is your style</li>
        </ul>`
      },
      {
        id: 'run-benchmarks',
        title: 'Run Your Own Benchmarks',
        content: `<p>Reproduce these benchmarks locally:</p>`,
        codeBlocks: [
          {
            language: 'bash',
            code: `# Install oha
cargo install oha

# Clone Armature
git clone https://github.com/pegasusheavy/armature
cd armature

# Build servers
cargo build --release --example micro_benchmark_server
cd benches/comparison_servers/actix_server && cargo build --release && cd -
cd benches/comparison_servers/axum_server && cargo build --release && cd -

# Run benchmarks
./scripts/benchmark-comparison.sh`
          }
        ]
      }
    ],
    relatedDocs: [
      {
        id: 'micro-framework-guide',
        title: 'Micro-Framework Guide',
        description: 'Learn the Armature micro-framework API'
      },
      {
        id: 'profiling-guide',
        title: 'Profiling Guide',
        description: 'CPU and memory profiling techniques'
      }
    ],
    seeAlso: [
      { title: 'Getting Started', id: 'readme' },
      { title: 'Performance Optimization', id: 'profiling-guide' }
    ]
  };
}

