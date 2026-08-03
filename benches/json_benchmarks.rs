//! JSON Serialization/Deserialization Benchmarks
//!
//! Compares serde_json (default) with simd-json (when feature enabled).
//!
//! Run benchmarks:
//!   cargo bench --bench json_benchmarks
//!   cargo bench --bench json_benchmarks --features simd-json

use armature_core::json;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use serde::{Deserialize, Serialize};
use std::hint::black_box;

// ============================================================================
// Test Data Structures
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SmallPayload {
    id: u64,
    name: String,
    active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MediumPayload {
    id: u64,
    username: String,
    email: String,
    first_name: String,
    last_name: String,
    age: u32,
    active: bool,
    roles: Vec<String>,
    metadata: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LargePayload {
    id: u64,
    username: String,
    email: String,
    profile: UserProfile,
    settings: UserSettings,
    posts: Vec<Post>,
    followers: Vec<u64>,
    following: Vec<u64>,
    metadata: std::collections::HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserProfile {
    bio: String,
    location: String,
    website: String,
    avatar_url: String,
    cover_url: String,
    verified: bool,
    joined_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserSettings {
    notifications_enabled: bool,
    email_notifications: bool,
    privacy_mode: String,
    theme: String,
    language: String,
    timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Post {
    id: u64,
    content: String,
    created_at: String,
    likes: u64,
    comments: u64,
    tags: Vec<String>,
}

// ============================================================================
// Test Data Generators
// ============================================================================

fn create_small_payload() -> SmallPayload {
    SmallPayload {
        id: 12345,
        name: "John Doe".to_string(),
        active: true,
    }
}

fn create_medium_payload() -> MediumPayload {
    MediumPayload {
        id: 12345,
        username: "johndoe".to_string(),
        email: "john.doe@example.com".to_string(),
        first_name: "John".to_string(),
        last_name: "Doe".to_string(),
        age: 30,
        active: true,
        roles: vec![
            "user".to_string(),
            "admin".to_string(),
            "moderator".to_string(),
        ],
        metadata: [
            ("signup_source".to_string(), "web".to_string()),
            ("referral".to_string(), "friend".to_string()),
            ("plan".to_string(), "premium".to_string()),
        ]
        .into_iter()
        .collect(),
    }
}

fn create_large_payload() -> LargePayload {
    LargePayload {
        id: 12345,
        username: "johndoe".to_string(),
        email: "john.doe@example.com".to_string(),
        profile: UserProfile {
            bio: "Software engineer passionate about Rust and web technologies. Building cool stuff.".to_string(),
            location: "San Francisco, CA".to_string(),
            website: "https://johndoe.dev".to_string(),
            avatar_url: "https://cdn.example.com/avatars/johndoe.png".to_string(),
            cover_url: "https://cdn.example.com/covers/johndoe.jpg".to_string(),
            verified: true,
            joined_at: "2020-01-15T10:30:00Z".to_string(),
        },
        settings: UserSettings {
            notifications_enabled: true,
            email_notifications: false,
            privacy_mode: "friends_only".to_string(),
            theme: "dark".to_string(),
            language: "en-US".to_string(),
            timezone: "America/Los_Angeles".to_string(),
        },
        posts: (0..10)
            .map(|i| Post {
                id: 1000 + i,
                content: format!("This is post number {}. It has some interesting content about technology, programming, and life in general. #rust #webdev", i),
                created_at: format!("2024-01-{:02}T12:00:00Z", i + 1),
                likes: (i * 17),
                comments: (i * 3),
                tags: vec!["rust".to_string(), "programming".to_string(), "tech".to_string()],
            })
            .collect(),
        followers: (0..100).collect(),
        following: (0..50).collect(),
        metadata: [
            ("last_login".to_string(), serde_json::json!("2024-01-20T15:30:00Z")),
            ("login_count".to_string(), serde_json::json!(1234)),
            ("features".to_string(), serde_json::json!(["beta", "premium", "early_access"])),
        ]
        .into_iter()
        .collect(),
    }
}

// ============================================================================
// Serialization Benchmarks
// ============================================================================

fn bench_serialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_serialize");

    // Small payload
    let small = create_small_payload();
    let small_bytes = json::to_vec(&small).unwrap();
    group.throughput(Throughput::Bytes(small_bytes.len() as u64));
    group.bench_function("small", |b| {
        b.iter(|| json::to_vec(black_box(&small)).unwrap())
    });

    // Medium payload
    let medium = create_medium_payload();
    let medium_bytes = json::to_vec(&medium).unwrap();
    group.throughput(Throughput::Bytes(medium_bytes.len() as u64));
    group.bench_function("medium", |b| {
        b.iter(|| json::to_vec(black_box(&medium)).unwrap())
    });

    // Large payload
    let large = create_large_payload();
    let large_bytes = json::to_vec(&large).unwrap();
    group.throughput(Throughput::Bytes(large_bytes.len() as u64));
    group.bench_function("large", |b| {
        b.iter(|| json::to_vec(black_box(&large)).unwrap())
    });

    group.finish();
}

// ============================================================================
// Deserialization Benchmarks
// ============================================================================

fn bench_deserialization(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_deserialize");

    // Small payload
    let small = create_small_payload();
    let small_bytes = serde_json::to_vec(&small).unwrap();
    group.throughput(Throughput::Bytes(small_bytes.len() as u64));
    group.bench_function("small", |b| {
        b.iter(|| {
            let _: SmallPayload = json::from_slice(black_box(&small_bytes)).unwrap();
        })
    });

    // Medium payload
    let medium = create_medium_payload();
    let medium_bytes = serde_json::to_vec(&medium).unwrap();
    group.throughput(Throughput::Bytes(medium_bytes.len() as u64));
    group.bench_function("medium", |b| {
        b.iter(|| {
            let _: MediumPayload = json::from_slice(black_box(&medium_bytes)).unwrap();
        })
    });

    // Large payload
    let large = create_large_payload();
    let large_bytes = serde_json::to_vec(&large).unwrap();
    group.throughput(Throughput::Bytes(large_bytes.len() as u64));
    group.bench_function("large", |b| {
        b.iter(|| {
            let _: LargePayload = json::from_slice(black_box(&large_bytes)).unwrap();
        })
    });

    group.finish();
}

// ============================================================================
// Roundtrip Benchmarks (Serialize + Deserialize)
// ============================================================================

fn bench_roundtrip(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_roundtrip");

    // Small payload
    let small = create_small_payload();
    let small_bytes = json::to_vec(&small).unwrap();
    group.throughput(Throughput::Bytes(small_bytes.len() as u64 * 2)); // *2 for roundtrip
    group.bench_function("small", |b| {
        b.iter(|| {
            let bytes = json::to_vec(black_box(&small)).unwrap();
            let _: SmallPayload = json::from_slice(&bytes).unwrap();
        })
    });

    // Medium payload
    let medium = create_medium_payload();
    let medium_bytes = json::to_vec(&medium).unwrap();
    group.throughput(Throughput::Bytes(medium_bytes.len() as u64 * 2));
    group.bench_function("medium", |b| {
        b.iter(|| {
            let bytes = json::to_vec(black_box(&medium)).unwrap();
            let _: MediumPayload = json::from_slice(&bytes).unwrap();
        })
    });

    // Large payload
    let large = create_large_payload();
    let large_bytes = json::to_vec(&large).unwrap();
    group.throughput(Throughput::Bytes(large_bytes.len() as u64 * 2));
    group.bench_function("large", |b| {
        b.iter(|| {
            let bytes = json::to_vec(black_box(&large)).unwrap();
            let _: LargePayload = json::from_slice(&bytes).unwrap();
        })
    });

    group.finish();
}

// ============================================================================
// HTTP Request/Response Simulation
// ============================================================================

fn bench_http_json(c: &mut Criterion) {
    use armature_core::{HttpRequest, HttpResponse};

    let mut group = c.benchmark_group("http_json");

    // Simulate parsing JSON from request body
    let medium = create_medium_payload();
    let body_bytes = serde_json::to_vec(&medium).unwrap();
    group.throughput(Throughput::Bytes(body_bytes.len() as u64));

    group.bench_function("request_parse", |b| {
        b.iter(|| {
            let mut req = HttpRequest::new("POST", "/api/users".to_string());
            req.body = armature_core::Bytes::from(body_bytes.clone());
            let _: MediumPayload = req.json().unwrap();
        })
    });

    // Simulate serializing JSON for response
    group.bench_function("response_json", |b| {
        b.iter(|| {
            let response = HttpResponse::ok().with_json(black_box(&medium)).unwrap();
            let _ = response.body;
        })
    });

    // Full request -> process -> response cycle
    group.bench_function("full_cycle", |b| {
        b.iter(|| {
            // Parse request
            let mut req = HttpRequest::new("POST", "/api/users".to_string());
            req.body = armature_core::Bytes::from(body_bytes.clone());
            let _user: MediumPayload = req.json().unwrap();

            // Create response (simulating transformed data)
            let response_data = MediumPayload {
                id: 99999,
                ..medium.clone()
            };
            HttpResponse::created()
                .with_json(black_box(&response_data))
                .unwrap()
        })
    });

    group.finish();
}

// ============================================================================
// Library Info
// ============================================================================

fn bench_library_info(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_info");

    // Report which library is being used
    println!("\n===========================================");
    println!("JSON Library: {}", json::library_name());
    println!("SIMD Enabled: {}", json::is_simd_enabled());
    println!("===========================================\n");

    // Quick sanity check that library works
    group.bench_function("library_check", |b| {
        b.iter(|| {
            let _ = json::library_name();
            json::is_simd_enabled()
        })
    });

    group.finish();
}

criterion_group!(
    json_benches,
    bench_library_info,
    bench_serialization,
    bench_deserialization,
    bench_roundtrip,
    bench_http_json,
);

criterion_main!(json_benches);
