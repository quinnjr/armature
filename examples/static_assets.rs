#![allow(dead_code)]
use armature_core::handler::from_legacy_handler;
use armature_core::*;
use armature_proc_macro::*;
use std::sync::Arc;
use std::time::Duration;

#[injectable]
#[derive(Clone, Default)]
struct ApiService;

impl ApiService {
    fn new() -> Self {
        Self
    }
}

#[controller("/api")]
struct ApiController {
    service: ApiService,
}

#[routes]
impl ApiController {
    #[get("/data")]
    async fn get_data(&self, _req: HttpRequest) -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_json(&serde_json::json!({
            "message": "Hello from API",
            "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
        }))
    }
}

#[module(
    providers: [ApiService],
    controllers: [ApiController]
)]
#[derive(Default, Clone)]
struct AppModule;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("🗂️  Armature Static Assets Example");
    println!("====================================\n");

    // Create a demo public directory structure
    println!("📁 Setting up demo directory structure...");
    setup_demo_directory().await?;

    let container = Container::new();
    let mut router = Router::new();

    // Example 1: Basic static asset serving with compression
    println!("\n📦 Example 1: Basic Static Assets (with Gzip compression)");
    println!("   URL: http://localhost:3000/static/");

    let basic_config = StaticAssetsConfig::new("demo/public")
        .with_default_strategy(CacheStrategy::Public(Duration::from_secs(3600)))
        .with_compression(
            CompressionConfig::new()
                .with_level(CompressionLevel::Default)
                .prefer_brotli(false), // Prefer gzip for compatibility
        );

    let basic_server = StaticAssetServer::new(basic_config)?;

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/static/*".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let server = basic_server.clone();
            Box::pin(async move {
                let path = req.path.trim_start_matches("/static");
                server
                    .serve(&HttpRequest::new("GET", path.to_string()))
                    .await
            })
        })),
        constraints: None,
    });

    // Example 2: SPA mode (fallback to index.html)
    println!("📦 Example 2: SPA Mode with Fallback");
    println!("   URL: http://localhost:3000/app/");

    let spa_config = StaticAssetsConfig::new("demo/spa").spa_mode(); // Automatically configures for SPAs

    let spa_server = StaticAssetServer::new(spa_config)?;

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/app/*".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let server = spa_server.clone();
            Box::pin(async move {
                let path = req.path.trim_start_matches("/app");
                let mut spa_req = HttpRequest::new("GET", path.to_string());
                // Copy headers for conditional requests
                spa_req.headers = req.headers.clone();
                server.serve(&spa_req).await
            })
        })),
        constraints: None,
    });

    // Example 3: Maximum performance (immutable assets + Brotli)
    println!("📦 Example 3: Maximum Performance Mode (Brotli + Best compression)");
    println!("   URL: http://localhost:3000/cdn/");

    let cdn_config = StaticAssetsConfig::new("demo/cdn")
        .max_performance() // Aggressive caching + best compression
        .with_cors_origin("*");

    let cdn_server = StaticAssetServer::new(cdn_config)?;

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/cdn/*".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let server = cdn_server.clone();
            Box::pin(async move {
                let path = req.path.trim_start_matches("/cdn");
                let mut cdn_req = HttpRequest::new("GET", path.to_string());
                cdn_req.headers = req.headers.clone();
                server.serve(&cdn_req).await
            })
        })),
        constraints: None,
    });

    // Example 4: Development mode (no caching)
    println!("📦 Example 4: Development Mode (No Caching)");
    println!("   URL: http://localhost:3000/dev/");

    let dev_config = StaticAssetsConfig::new("demo/dev").development(); // No caching for development

    let dev_server = StaticAssetServer::new(dev_config)?;

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/dev/*".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let server = dev_server.clone();
            Box::pin(async move {
                let path = req.path.trim_start_matches("/dev");
                server
                    .serve(&HttpRequest::new("GET", path.to_string()))
                    .await
            })
        })),
        constraints: None,
    });

    // Example 5: Custom per-filetype caching
    println!("📦 Example 5: Custom Per-FileType Caching");
    println!("   URL: http://localhost:3000/custom/");

    let custom_config = StaticAssetsConfig::new("demo/custom")
        .with_type_strategy(FileType::JavaScript, CacheStrategy::Immutable)
        .with_type_strategy(FileType::Stylesheet, CacheStrategy::Immutable)
        .with_type_strategy(
            FileType::Image,
            CacheStrategy::Public(Duration::from_secs(86400)),
        )
        .with_type_strategy(FileType::Html, CacheStrategy::NoCache)
        .with_etag(true)
        .with_last_modified(true);

    let custom_server = StaticAssetServer::new(custom_config)?;

    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/custom/*".to_string(),
        handler: from_legacy_handler(Arc::new(move |req: HttpRequest| {
            let server = custom_server.clone();
            Box::pin(async move {
                let path = req.path.trim_start_matches("/custom");
                let mut custom_req = HttpRequest::new("GET", path.to_string());
                custom_req.headers = req.headers.clone();
                server.serve(&custom_req).await
            })
        })),
        constraints: None,
    });

    // API route for comparison
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/api/data".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async {
                HttpResponse::ok().with_json(&serde_json::json!({
                    "message": "This is an API endpoint, not a static asset",
                    "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
                }))
            })
        })),
        constraints: None,
    });

    // Info page
    router.add_route(Route {
        method: HttpMethod::GET,
        path: "/".to_string(),
        handler: from_legacy_handler(Arc::new(|_req: HttpRequest| {
            Box::pin(async {
                let html = r#"
<!DOCTYPE html>
<html>
<head>
    <title>Static Assets Demo</title>
    <style>
        body { font-family: Arial, sans-serif; max-width: 1200px; margin: 50px auto; padding: 20px; }
        h1 { color: #333; }
        .example { background: #f5f5f5; padding: 20px; margin: 20px 0; border-radius: 5px; }
        .example h2 { margin-top: 0; color: #007bff; }
        .example ul { margin: 10px 0; }
        .example li { margin: 5px 0; }
        code { background: #e0e0e0; padding: 2px 6px; border-radius: 3px; }
        .cache-info { font-size: 0.9em; color: #666; }
    </style>
</head>
<body>
    <h1>🗂️ Armature Static Assets Demo</h1>
    <p>This example demonstrates different static asset serving strategies with configurable caching.</p>

    <div class="example">
        <h2>📦 Example 1: Basic Static Assets</h2>
        <p><strong>URL:</strong> <code>/static/*</code></p>
        <p class="cache-info">Cache: Public, 1 hour max-age | Compression: Gzip (default)</p>
        <ul>
            <li><a href="/static/index.html">index.html</a> - Main page (compressed)</li>
            <li><a href="/static/styles.css">styles.css</a> - Stylesheet (compressed)</li>
            <li><a href="/static/script.js">script.js</a> - JavaScript (compressed)</li>
            <li><a href="/static/logo.svg">logo.svg</a> - Image (not compressed)</li>
        </ul>
    </div>

    <div class="example">
        <h2>📦 Example 2: SPA Mode</h2>
        <p><strong>URL:</strong> <code>/app/*</code></p>
        <p class="cache-info">Cache: Fallback to index.html, immutable JS/CSS, no-cache HTML</p>
        <ul>
            <li><a href="/app/">Root</a> → Falls back to index.html</li>
            <li><a href="/app/dashboard">Dashboard route</a> → Falls back to index.html</li>
            <li><a href="/app/unknown-route">Unknown route</a> → Falls back to index.html</li>
        </ul>
    </div>

    <div class="example">
        <h2>📦 Example 3: CDN Mode (Max Performance)</h2>
        <p><strong>URL:</strong> <code>/cdn/*</code></p>
        <p class="cache-info">Cache: Immutable, 1 year max-age | Compression: Brotli (Best), CORS enabled</p>
        <ul>
            <li><a href="/cdn/bundle.123456.js">bundle.123456.js</a> - Hashed JS (Brotli compressed)</li>
            <li><a href="/cdn/styles.abc123.css">styles.abc123.css</a> - Hashed CSS (Brotli compressed)</li>
        </ul>
        <p class="cache-info">💡 Check Content-Encoding header for "br" (Brotli)</p>
    </div>

    <div class="example">
        <h2>📦 Example 4: Development Mode</h2>
        <p><strong>URL:</strong> <code>/dev/*</code></p>
        <p class="cache-info">Cache: No caching, no ETags | Compression: Disabled</p>
        <ul>
            <li><a href="/dev/test.js">test.js</a> - Always fresh, uncompressed</li>
            <li><a href="/dev/test.css">test.css</a> - Always fresh, uncompressed</li>
        </ul>
    </div>

    <div class="example">
        <h2>📦 Example 5: Custom Caching</h2>
        <p><strong>URL:</strong> <code>/custom/*</code></p>
        <p class="cache-info">Cache: Mixed strategies, ETags enabled</p>
        <ul>
            <li>JS/CSS: Immutable</li>
            <li>Images: 24 hours</li>
            <li>HTML: No cache</li>
        </ul>
    </div>

    <div class="example">
        <h2>🔍 Testing Cache & Compression Headers</h2>
        <p>Use browser DevTools (Network tab) or curl to inspect headers:</p>
        <pre><code># Test cache headers
curl -I http://localhost:3000/static/script.js

# Test gzip compression
curl -H "Accept-Encoding: gzip" -I http://localhost:3000/static/script.js

# Test brotli compression
curl -H "Accept-Encoding: br" -I http://localhost:3000/cdn/bundle.123456.js

# Test conditional request (ETag)
curl -H "If-None-Match: <etag-value>" http://localhost:3000/static/script.js

# Download compressed content
curl -H "Accept-Encoding: gzip" http://localhost:3000/static/script.js --output script.js.gz</code></pre>
    </div>

    <div class="example">
        <h2>🎯 Key Features</h2>
        <ul>
            <li>✅ Multiple cache strategies (NoCache, Public, Private, Immutable)</li>
            <li>✅ Per-filetype cache configuration</li>
            <li>✅ <strong>Gzip and Brotli compression</strong></li>
            <li>✅ <strong>Configurable compression levels (Fast, Default, Best)</strong></li>
            <li>✅ <strong>Pre-compressed file support (.gz, .br)</strong></li>
            <li>✅ <strong>Smart compression (only compressible files, size limits)</strong></li>
            <li>✅ ETag support for conditional requests</li>
            <li>✅ Last-Modified headers</li>
            <li>✅ 304 Not Modified responses</li>
            <li>✅ SPA fallback routing</li>
            <li>✅ CORS support</li>
            <li>✅ Path traversal prevention</li>
            <li>✅ Automatic Content-Type detection</li>
        </ul>
    </div>
</body>
</html>
                "#;
                Ok(HttpResponse::ok()
                    .with_header("Content-Type".to_string(), "text/html".to_string())
                    .with_body(html.as_bytes().to_vec()))
            })
        })),
        constraints: None,
    });

    println!("\n🚀 Server starting on http://localhost:3000");
    println!("\nExamples:");
    println!("  • http://localhost:3000/              → Info page");
    println!("  • http://localhost:3000/static/       → Basic static files");
    println!("  • http://localhost:3000/app/          → SPA mode");
    println!("  • http://localhost:3000/cdn/          → CDN mode");
    println!("  • http://localhost:3000/dev/          → Dev mode");
    println!("  • http://localhost:3000/custom/       → Custom caching");
    println!("  • http://localhost:3000/api/data      → API endpoint\n");

    let app = Application::new(container, router);

    app.listen(3000).await?;

    Ok(())
}

/// Setup demo directory structure with sample files
async fn setup_demo_directory() -> std::io::Result<()> {
    use tokio::fs;

    // Create directories
    fs::create_dir_all("demo/public").await?;
    fs::create_dir_all("demo/spa").await?;
    fs::create_dir_all("demo/cdn").await?;
    fs::create_dir_all("demo/dev").await?;
    fs::create_dir_all("demo/custom").await?;

    // Sample files for basic static serving (make them large enough to compress)
    let large_html = format!(
        "<html><body><h1>Static Index</h1>{}</body></html>",
        "<!-- Padding to make file larger -->".repeat(100)
    );
    fs::write("demo/public/index.html", large_html).await?;

    let large_css = format!(
        "body {{ margin: 0; padding: 20px; }} {}",
        "/* Padding comment */\n.class {} ".repeat(100)
    );
    fs::write("demo/public/styles.css", large_css).await?;

    let large_js = format!(
        "console.log('Static script loaded');\n{}",
        "// Padding comment\nfunction noop() {}\n".repeat(100)
    );
    fs::write("demo/public/script.js", large_js).await?;
    fs::write("demo/public/logo.svg", "<svg></svg>").await?;

    // SPA files
    let spa_html = format!(
        "<html><body><div id=\"app\">SPA Root</div>{}</body></html>",
        "<!-- SPA padding -->".repeat(50)
    );
    fs::write("demo/spa/index.html", spa_html).await?;

    // CDN files (with hashed names) - large for good compression
    let cdn_js = format!(
        "// Hashed bundle\n{}",
        "function module() {{ return 'code'; }}\n".repeat(200)
    );
    fs::write("demo/cdn/bundle.123456.js", cdn_js).await?;

    let cdn_css = format!(
        "/* Hashed styles */\n{}",
        ".component {{ display: block; }}\n".repeat(200)
    );
    fs::write("demo/cdn/styles.abc123.css", cdn_css).await?;

    // Dev files
    fs::write(
        "demo/dev/test.js",
        "// Development file\nconsole.log('dev');",
    )
    .await?;
    fs::write(
        "demo/dev/test.css",
        "/* Development styles */\nbody { color: red; }",
    )
    .await?;

    // Custom files
    fs::write(
        "demo/custom/index.html",
        "<html><body><h1>Custom</h1></body></html>",
    )
    .await?;
    fs::write("demo/custom/app.js", "// Custom JS\nconsole.log('custom');").await?;

    println!("✅ Demo files created");

    Ok(())
}
