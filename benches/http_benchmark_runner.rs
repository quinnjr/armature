#![allow(deprecated)]
#![allow(clippy::needless_question_mark)]

//! HTTP Benchmark Runner
//!
//! A comprehensive HTTP benchmark tool that can compare Armature with other
//! Rust web frameworks using real HTTP traffic.
//!
//! ## Usage
//!
//! ```bash
//! # Run Armature benchmarks
//! cargo run --release --bin http-benchmark -- --framework armature
//!
//! # Run all framework benchmarks (requires separate servers)
//! cargo run --release --bin http-benchmark -- --all
//!
//! # Custom configuration
//! cargo run --release --bin http-benchmark -- --duration 30 --connections 100 --threads 4
//! ```
//!
//! ## Requirements
//!
//! For full framework comparisons, you need:
//! - `wrk` or `oha` installed for HTTP load testing
//! - Each framework's example server running on its designated port
//!
//! ## Ports
//!
//! - Armature: 3000
//! - Actix-web: 3001
//! - Axum: 3002
//! - Warp: 3003
//! - Rocket: 3004
//! - Next.js: 3005
//! - Express: 3006
//! - Koa: 3007
//! - NestJS: 3008

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// Framework configuration for benchmarking
#[derive(Debug, Clone)]
pub struct FrameworkConfig {
    pub name: &'static str,
    pub port: u16,
    pub description: &'static str,
}

/// Benchmark result for a single test
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    pub framework: String,
    pub endpoint: String,
    pub requests_per_second: f64,
    pub latency_avg_ms: f64,
    pub latency_p50_ms: f64,
    pub latency_p90_ms: f64,
    pub latency_p99_ms: f64,
    pub total_requests: u64,
    pub errors: u64,
    pub transfer_rate_mbps: f64,
}

/// Benchmark configuration
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    pub duration_secs: u32,
    pub connections: u32,
    pub threads: u32,
    pub warmup_secs: u32,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            duration_secs: 10,
            connections: 50,
            threads: 4,
            warmup_secs: 2,
        }
    }
}

/// Available frameworks with their configurations
pub fn get_frameworks() -> Vec<FrameworkConfig> {
    vec![
        FrameworkConfig {
            name: "armature",
            port: 3000,
            description: "Armature - NestJS-inspired Rust framework",
        },
        FrameworkConfig {
            name: "actix-web",
            port: 3001,
            description: "Actix-web - High-performance actor-based framework",
        },
        FrameworkConfig {
            name: "axum",
            port: 3002,
            description: "Axum - Tower-based modular framework",
        },
        FrameworkConfig {
            name: "warp",
            port: 3003,
            description: "Warp - Filter-based composable framework",
        },
        FrameworkConfig {
            name: "rocket",
            port: 3004,
            description: "Rocket - Easy-to-use web framework",
        },
        FrameworkConfig {
            name: "nextjs",
            port: 3005,
            description: "Next.js - React framework with API routes",
        },
        FrameworkConfig {
            name: "express",
            port: 3006,
            description: "Express - Minimal Node.js web framework",
        },
        FrameworkConfig {
            name: "koa",
            port: 3007,
            description: "Koa - Next-gen Node.js web framework",
        },
        FrameworkConfig {
            name: "nestjs",
            port: 3008,
            description: "NestJS - Progressive Node.js framework",
        },
    ]
}

/// Benchmark endpoints to test
/// Endpoint definition with paths for different frameworks
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub name: &'static str,
    pub method: &'static str,
    pub rust_path: &'static str,   // Path for Rust frameworks
    pub nextjs_path: &'static str, // Path for Next.js (with /api prefix)
}

pub fn get_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint {
            name: "plaintext",
            method: "GET",
            rust_path: "/",
            nextjs_path: "/api",
        },
        Endpoint {
            name: "json",
            method: "GET",
            rust_path: "/json",
            nextjs_path: "/api/json",
        },
        Endpoint {
            name: "param",
            method: "GET",
            rust_path: "/users/123",
            nextjs_path: "/api/users/123",
        },
        Endpoint {
            name: "json_post",
            method: "POST",
            rust_path: "/api/users",
            nextjs_path: "/api/users",
        },
        // No `data_medium` here. `/data?size=medium` is served only by
        // `examples/benchmark_server.rs`; actix/axum/warp/rocket and the Node
        // servers all 404 it, so including it would rank Armature's real
        // payload against four 404 pages. A comparison endpoint has to exist on
        // every framework being compared.
    ]
}

/// Fraction of non-2xx responses above which a result is not comparable.
///
/// A run that mostly 404s still reports a high req/s - fast enough to look like
/// a win - so results past this threshold are reported and discarded rather
/// than ranked.
const MAX_NON_2XX_RATIO: f64 = 0.01;

/// Get the correct path for a framework
fn get_endpoint_path<'a>(endpoint: &'a Endpoint, framework: &str) -> &'a str {
    // Next.js uses /api prefix for all routes
    if framework == "nextjs" {
        endpoint.nextjs_path
    } else {
        // Rust frameworks and Express/Koa/NestJS use standard paths
        endpoint.rust_path
    }
}

/// Check if wrk or oha is available
pub fn find_benchmark_tool() -> Option<&'static str> {
    // Try oha first (faster, Rust-based)
    if Command::new("oha")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return Some("oha");
    }

    // Fallback to wrk
    if Command::new("wrk")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return Some("wrk");
    }

    None
}

/// Run benchmark using oha
pub fn run_oha_benchmark(
    url: &str,
    config: &BenchmarkConfig,
    method: &str,
    body: Option<&str>,
) -> Result<BenchmarkResult, String> {
    let mut args = vec![
        "-z".to_string(),
        format!("{}s", config.duration_secs),
        "-c".to_string(),
        config.connections.to_string(),
        "-m".to_string(),
        method.to_string(),
        "--no-tui".to_string(),
        "-j".to_string(), // JSON output
    ];

    if let Some(b) = body {
        args.push("-d".to_string());
        args.push(b.to_string());
        args.push("-H".to_string());
        args.push("Content-Type: application/json".to_string());
    }

    args.push(url.to_string());

    let output = Command::new("oha")
        .args(&args)
        .output()
        .map_err(|e| format!("Failed to run oha: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "oha failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_oha_output(&stdout, url)
}

/// Parse oha JSON output
fn parse_oha_output(output: &str, url: &str) -> Result<BenchmarkResult, String> {
    let json: serde_json::Value =
        serde_json::from_str(output).map_err(|e| format!("Failed to parse oha output: {}", e))?;

    let summary = json.get("summary").ok_or("Missing summary")?;

    let transport_errors: u64 = summary["errorDistribution"]
        .as_object()
        .map(|m| m.values().filter_map(|v| v.as_u64()).sum())
        .unwrap_or(0);
    let non_2xx = count_non_2xx(&json);

    Ok(BenchmarkResult {
        framework: "unknown".to_string(),
        endpoint: url.to_string(),
        requests_per_second: summary["requestsPerSec"].as_f64().unwrap_or(0.0),
        latency_avg_ms: summary["average"]
            .as_f64()
            .map(|v| v * 1000.0)
            .unwrap_or(0.0),
        latency_p50_ms: summary["percentiles"]["p50"]
            .as_f64()
            .map(|v| v * 1000.0)
            .unwrap_or(0.0),
        latency_p90_ms: summary["percentiles"]["p90"]
            .as_f64()
            .map(|v| v * 1000.0)
            .unwrap_or(0.0),
        latency_p99_ms: summary["percentiles"]["p99"]
            .as_f64()
            .map(|v| v * 1000.0)
            .unwrap_or(0.0),
        total_requests: summary["total"].as_u64().unwrap_or(0),
        // `errorDistribution` counts transport failures only. A server that
        // answers every request with a 404 produces zero entries there, so the
        // status-code histogram has to be folded in as well or a 100%-404 run
        // reads as clean throughput.
        errors: transport_errors + non_2xx,
        transfer_rate_mbps: 0.0, // oha doesn't report this directly
    })
}

/// Count responses outside the 2xx range in oha's status-code histogram.
fn count_non_2xx(json: &serde_json::Value) -> u64 {
    json.get("statusCodeDistribution")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter(|(code, _)| !matches!(code.parse::<u16>(), Ok(200..=299)))
                .filter_map(|(_, count)| count.as_u64())
                .sum()
        })
        .unwrap_or(0)
}

/// Escape a string for embedding in a double-quoted Lua literal.
fn lua_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

/// Build the wrk Lua script that pins the method, body and content type.
///
/// wrk has no `-m`/`-d` flags; the only way to send anything but a bodyless GET
/// is a `-s` script. Without one, a POST endpoint is silently benchmarked as a
/// GET with no payload and still reported as a POST.
fn wrk_script(method: &str, body: Option<&str>) -> String {
    let mut script = format!("wrk.method = \"{}\"\n", lua_escape(method));
    if let Some(b) = body {
        script.push_str(&format!("wrk.body = \"{}\"\n", lua_escape(b)));
        script.push_str("wrk.headers[\"Content-Type\"] = \"application/json\"\n");
    }
    script
}

/// Run benchmark using wrk
pub fn run_wrk_benchmark(
    url: &str,
    config: &BenchmarkConfig,
    method: &str,
    body: Option<&str>,
) -> Result<BenchmarkResult, String> {
    let duration = format!("{}s", config.duration_secs);
    let threads = config.threads.to_string();
    let connections = config.connections.to_string();

    let mut args: Vec<String> = vec![
        "-t".into(),
        threads,
        "-c".into(),
        connections,
        "-d".into(),
        duration,
        "--latency".into(),
    ];

    // A bodyless GET is wrk's default, so it needs no script. Anything else
    // does, or the request that actually goes on the wire will not be the one
    // the result claims to measure.
    let script_path = if method.eq_ignore_ascii_case("GET") && body.is_none() {
        None
    } else {
        let path = std::env::temp_dir().join(format!(
            "armature-wrk-{}-{}.lua",
            std::process::id(),
            method.to_ascii_lowercase()
        ));
        std::fs::write(&path, wrk_script(method, body))
            .map_err(|e| format!("Failed to write wrk script {}: {}", path.display(), e))?;
        args.push("-s".into());
        args.push(path.display().to_string());
        Some(path)
    };

    args.push(url.to_string());

    let output = Command::new("wrk").args(&args).output();

    if let Some(path) = &script_path {
        let _ = std::fs::remove_file(path);
    }

    let output = output.map_err(|e| format!("Failed to run wrk: {}", e))?;

    if !output.status.success() {
        return Err(format!(
            "wrk failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_wrk_output(&stdout, url)
}

/// Parse wrk output
fn parse_wrk_output(output: &str, url: &str) -> Result<BenchmarkResult, String> {
    let mut result = BenchmarkResult {
        framework: "unknown".to_string(),
        endpoint: url.to_string(),
        requests_per_second: 0.0,
        latency_avg_ms: 0.0,
        latency_p50_ms: 0.0,
        latency_p90_ms: 0.0,
        latency_p99_ms: 0.0,
        total_requests: 0,
        errors: 0,
        transfer_rate_mbps: 0.0,
    };

    for line in output.lines() {
        let line = line.trim();

        if line.starts_with("Requests/sec:") {
            if let Some(val) = line.split_whitespace().nth(1) {
                result.requests_per_second = val.parse().unwrap_or(0.0);
            }
        } else if line.starts_with("Latency") && !line.contains("Distribution") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                result.latency_avg_ms = parse_duration_to_ms(parts[1]);
            }
        } else if line.contains("50%") {
            if let Some(val) = line.split_whitespace().nth(1) {
                result.latency_p50_ms = parse_duration_to_ms(val);
            }
        } else if line.contains("90%") {
            if let Some(val) = line.split_whitespace().nth(1) {
                result.latency_p90_ms = parse_duration_to_ms(val);
            }
        } else if line.contains("99%") {
            if let Some(val) = line.split_whitespace().nth(1) {
                result.latency_p99_ms = parse_duration_to_ms(val);
            }
        } else if line.contains("requests in") {
            if let Some(val) = line.split_whitespace().next() {
                result.total_requests = val.parse().unwrap_or(0);
            }
        } else if line.starts_with("Transfer/sec:") {
            if let Some(val) = line.split_whitespace().nth(1) {
                result.transfer_rate_mbps = parse_transfer_rate(val);
            }
        } else if line.contains("Socket errors:") || line.contains("Non-2xx") {
            // Count errors
            for part in line.split(',') {
                if let Some(num) = part.split_whitespace().last() {
                    result.errors += num.parse::<u64>().unwrap_or(0);
                }
            }
        }
    }

    Ok(result)
}

fn parse_duration_to_ms(s: &str) -> f64 {
    let s = s.trim();
    if let Some(stripped) = s.strip_suffix("ms") {
        stripped.parse().unwrap_or(0.0)
    } else if let Some(stripped) = s.strip_suffix("us") {
        stripped.parse::<f64>().unwrap_or(0.0) / 1000.0
    } else if let Some(stripped) = s.strip_suffix('s') {
        stripped.parse::<f64>().unwrap_or(0.0) * 1000.0
    } else {
        s.parse().unwrap_or(0.0)
    }
}

fn parse_transfer_rate(s: &str) -> f64 {
    let s = s.trim();
    if let Some(stripped) = s.strip_suffix("MB") {
        stripped.parse().unwrap_or(0.0)
    } else if let Some(stripped) = s.strip_suffix("KB") {
        stripped.parse::<f64>().unwrap_or(0.0) / 1024.0
    } else if let Some(stripped) = s.strip_suffix("GB") {
        stripped.parse::<f64>().unwrap_or(0.0) * 1024.0
    } else {
        s.parse().unwrap_or(0.0)
    }
}

/// Check if a server is running on a port
pub fn check_server_running(port: u16) -> bool {
    std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok()
}

/// Print results as a table
pub fn print_results_table(results: &[BenchmarkResult]) {
    println!("\n{:=<100}", "");
    println!("{:^100}", "BENCHMARK RESULTS");
    println!("{:=<100}", "");

    println!(
        "\n{:<15} {:<20} {:>12} {:>10} {:>10} {:>10} {:>10}",
        "Framework", "Endpoint", "Req/s", "Avg (ms)", "p50 (ms)", "p90 (ms)", "p99 (ms)"
    );
    println!("{:-<97}", "");

    for result in results {
        println!(
            "{:<15} {:<20} {:>12.2} {:>10.2} {:>10.2} {:>10.2} {:>10.2}",
            result.framework,
            truncate(&result.endpoint, 20),
            result.requests_per_second,
            result.latency_avg_ms,
            result.latency_p50_ms,
            result.latency_p90_ms,
            result.latency_p99_ms
        );
    }

    println!("{:-<97}", "");
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len - 3])
    }
}

/// Print comparison summary
pub fn print_comparison_summary(results: &[BenchmarkResult]) {
    // Group by endpoint
    let mut by_endpoint: HashMap<String, Vec<&BenchmarkResult>> = HashMap::new();
    for r in results {
        by_endpoint.entry(r.endpoint.clone()).or_default().push(r);
    }

    println!("\n{:=<100}", "");
    println!("{:^100}", "COMPARISON SUMMARY");
    println!("{:=<100}\n", "");

    for (endpoint, endpoint_results) in &by_endpoint {
        println!("📊 {}", endpoint);
        println!("{:-<60}", "");

        // Sort by requests per second (descending)
        let mut sorted: Vec<_> = endpoint_results.iter().collect();
        sorted.sort_by(|a, b| {
            b.requests_per_second
                .partial_cmp(&a.requests_per_second)
                .unwrap()
        });

        if let Some(fastest) = sorted.first() {
            for (i, r) in sorted.iter().enumerate() {
                let relative = if fastest.requests_per_second > 0.0 {
                    (r.requests_per_second / fastest.requests_per_second) * 100.0
                } else {
                    0.0
                };

                let medal = match i {
                    0 => "🥇",
                    1 => "🥈",
                    2 => "🥉",
                    _ => "  ",
                };

                println!(
                    "{} {:<15} {:>10.0} req/s ({:>5.1}%) | p99: {:>6.2}ms",
                    medal, r.framework, r.requests_per_second, relative, r.latency_p99_ms
                );
            }
        }
        println!();
    }
}

/// Generate markdown report
pub fn generate_markdown_report(results: &[BenchmarkResult], config: &BenchmarkConfig) -> String {
    let mut report = String::new();

    report.push_str("# Framework Comparison Benchmark Results\n\n");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    report.push_str(&format!("**Date:** {} (Unix timestamp)\n\n", now));
    report.push_str("## Configuration\n\n");
    report.push_str(&format!("- Duration: {} seconds\n", config.duration_secs));
    report.push_str(&format!("- Connections: {}\n", config.connections));
    report.push_str(&format!("- Threads: {}\n", config.threads));
    report.push_str(&format!("- Warmup: {} seconds\n\n", config.warmup_secs));

    report.push_str("## Results\n\n");
    report
        .push_str("| Framework | Endpoint | Req/s | Avg (ms) | p50 (ms) | p90 (ms) | p99 (ms) |\n");
    report.push_str("|-----------|----------|------:|--------:|--------:|--------:|--------:|\n");

    for r in results {
        report.push_str(&format!(
            "| {} | {} | {:.0} | {:.2} | {:.2} | {:.2} | {:.2} |\n",
            r.framework,
            r.endpoint,
            r.requests_per_second,
            r.latency_avg_ms,
            r.latency_p50_ms,
            r.latency_p90_ms,
            r.latency_p99_ms
        ));
    }

    report.push_str("\n## Methodology\n\n");
    report.push_str("All frameworks were tested with identical endpoints:\n\n");
    report.push_str("1. **Plaintext** (`/`) - Returns \"Hello, World!\"\n");
    report.push_str("2. **JSON** (`/json`) - Returns a small JSON object\n");
    report.push_str("3. **Path Parameter** (`/users/:id`) - Returns user data\n");
    report.push_str("4. **JSON POST** (`/api/users`) - Accepts and returns JSON\n\n");
    report.push_str(
        "Each test includes a warmup period to stabilize JIT compilation and connection pooling.\n",
    );

    report
}

/// Main benchmark runner
pub fn run_benchmarks(
    frameworks: &[FrameworkConfig],
    config: &BenchmarkConfig,
) -> Vec<BenchmarkResult> {
    let tool = find_benchmark_tool();
    if tool.is_none() {
        eprintln!("❌ No benchmark tool found! Install 'oha' (recommended) or 'wrk'.");
        eprintln!("   cargo install oha");
        eprintln!("   # or");
        eprintln!("   apt install wrk / brew install wrk");
        return vec![];
    }
    let tool = tool.unwrap();
    println!("🔧 Using benchmark tool: {}\n", tool);

    let endpoints = get_endpoints();
    let mut results = Vec::new();

    for framework in frameworks {
        println!(
            "\n📊 Benchmarking: {} (port {})",
            framework.name, framework.port
        );
        println!("   {}", framework.description);

        if !check_server_running(framework.port) {
            println!(
                "   ⚠️  Server not running on port {}, skipping...",
                framework.port
            );
            continue;
        }

        for endpoint in &endpoints {
            let path = get_endpoint_path(endpoint, framework.name);
            let url = format!("http://127.0.0.1:{}{}", framework.port, path);

            // Warmup
            print!("   Warming up {} ({})...", endpoint.name, path);
            let _ = std::io::Write::flush(&mut std::io::stdout());

            let body = if endpoint.method == "POST" {
                Some(r#"{"name":"John","email":"john@example.com"}"#)
            } else {
                None
            };

            let warmup_config = BenchmarkConfig {
                duration_secs: config.warmup_secs,
                connections: config.connections / 2,
                threads: config.threads,
                warmup_secs: 0,
            };

            let _ = match tool {
                "oha" => run_oha_benchmark(&url, &warmup_config, endpoint.method, body),
                "wrk" => run_wrk_benchmark(&url, &warmup_config, endpoint.method, body),
                _ => Err("Unknown tool".to_string()),
            };

            println!(" done");

            // Actual benchmark
            print!("   Running {} benchmark...", endpoint.name);
            let _ = std::io::Write::flush(&mut std::io::stdout());

            let result = match tool {
                "oha" => run_oha_benchmark(&url, config, endpoint.method, body),
                "wrk" => run_wrk_benchmark(&url, config, endpoint.method, body),
                _ => Err("Unknown tool".to_string()),
            };

            match result {
                Ok(mut r) => {
                    r.framework = framework.name.to_string();
                    r.endpoint = format!("{} {}", endpoint.method, endpoint.name);

                    // A framework that does not implement the endpoint answers
                    // fast and uniformly - which looks like a good score. Drop
                    // the result rather than rank a 404 against a real payload.
                    let error_ratio = if r.total_requests > 0 {
                        r.errors as f64 / r.total_requests as f64
                    } else {
                        0.0
                    };
                    if error_ratio > MAX_NON_2XX_RATIO {
                        println!(
                            " ⚠️  discarded: {}/{} responses were errors or non-2xx ({:.1}%)",
                            r.errors,
                            r.total_requests,
                            error_ratio * 100.0
                        );
                        continue;
                    }

                    println!(
                        " {:.0} req/s, p99: {:.2}ms",
                        r.requests_per_second, r.latency_p99_ms
                    );
                    results.push(r);
                }
                Err(e) => {
                    println!(" ❌ Error: {}", e);
                }
            }
        }
    }

    results
}

fn main() {
    use std::env;

    println!("╔═══════════════════════════════════════════════════════════════╗");
    println!("║         Armature Framework Comparison Benchmarks              ║");
    println!("╚═══════════════════════════════════════════════════════════════╝\n");

    let args: Vec<String> = env::args().collect();

    let mut config = BenchmarkConfig::default();
    let mut frameworks_to_test: Vec<String> = vec![];

    // Parse arguments
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--duration" | "-d" => {
                if i + 1 < args.len() {
                    config.duration_secs = args[i + 1].parse().unwrap_or(10);
                    i += 1;
                }
            }
            "--connections" | "-c" => {
                if i + 1 < args.len() {
                    config.connections = args[i + 1].parse().unwrap_or(50);
                    i += 1;
                }
            }
            "--threads" | "-t" => {
                if i + 1 < args.len() {
                    config.threads = args[i + 1].parse().unwrap_or(4);
                    i += 1;
                }
            }
            "--warmup" | "-w" => {
                if i + 1 < args.len() {
                    config.warmup_secs = args[i + 1].parse().unwrap_or(2);
                    i += 1;
                }
            }
            "--framework" | "-f" => {
                if i + 1 < args.len() {
                    frameworks_to_test.push(args[i + 1].clone());
                    i += 1;
                }
            }
            "--all" | "-a" => {
                frameworks_to_test = get_frameworks()
                    .iter()
                    .map(|f| f.name.to_string())
                    .collect();
            }
            "--help" | "-h" => {
                print_help();
                return;
            }
            _ => {}
        }
        i += 1;
    }

    // Default to Armature if no frameworks specified
    if frameworks_to_test.is_empty() {
        frameworks_to_test.push("armature".to_string());
    }

    // Filter frameworks
    let all_frameworks = get_frameworks();
    let frameworks: Vec<_> = all_frameworks
        .into_iter()
        .filter(|f| frameworks_to_test.contains(&f.name.to_string()))
        .collect();

    println!("Configuration:");
    println!("  Duration: {} seconds", config.duration_secs);
    println!("  Connections: {}", config.connections);
    println!("  Threads: {}", config.threads);
    println!("  Warmup: {} seconds", config.warmup_secs);
    println!(
        "  Frameworks: {}",
        frameworks
            .iter()
            .map(|f| f.name)
            .collect::<Vec<_>>()
            .join(", ")
    );

    let results = run_benchmarks(&frameworks, &config);

    if !results.is_empty() {
        print_results_table(&results);

        if frameworks.len() > 1 {
            print_comparison_summary(&results);
        }

        // Save markdown report
        let report = generate_markdown_report(&results, &config);
        let report_path = "benchmark_results.md";
        if std::fs::write(report_path, &report).is_ok() {
            println!("\n📄 Report saved to: {}", report_path);
        }
    }
}

fn print_help() {
    println!("Usage: http-benchmark [OPTIONS]");
    println!();
    println!("Options:");
    println!("  -d, --duration <SECS>     Test duration in seconds (default: 10)");
    println!("  -c, --connections <NUM>   Number of connections (default: 50)");
    println!("  -t, --threads <NUM>       Number of threads (default: 4)");
    println!("  -w, --warmup <SECS>       Warmup duration (default: 2)");
    println!("  -f, --framework <NAME>    Framework to test (can be used multiple times)");
    println!("  -a, --all                 Test all frameworks");
    println!("  -h, --help                Print help");
    println!();
    println!("Frameworks:");
    for f in get_frameworks() {
        println!("  {:15} - {} (port {})", f.name, f.description, f.port);
    }
    println!();
    println!("Examples:");
    println!("  http-benchmark -f armature");
    println!("  http-benchmark -f armature -f actix-web -d 30");
    println!("  http-benchmark --all -c 100 -t 8");
}
