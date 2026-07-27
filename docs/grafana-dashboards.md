# Grafana Dashboards

Pre-built Grafana dashboards for comprehensive monitoring of your Armature applications. Visualize HTTP metrics, authentication, caching, and background job processing in real-time.

## Table of Contents

- [Overview](#overview)
- [Available Dashboards](#available-dashboards)
- [Quick Start](#quick-start)
- [Application Overview Dashboard](#application-overview-dashboard)
- [Auth & Security Dashboard](#auth--security-dashboard)
- [Cache & Redis Dashboard](#cache--redis-dashboard)
- [Queues & Jobs Dashboard](#queues--jobs-dashboard)
- [Docker Compose Setup](#docker-compose-setup)
- [Setting Up Alerts](#setting-up-alerts)
- [Metric Reference](#metric-reference)
- [Troubleshooting](#troubleshooting)
- [Related Documentation](#related-documentation)

---

## Overview

- **Application Overview** - Request rates, latency, error rates, and resource usage at a glance
- **Auth & Security** - Login attempts, JWT tokens, rate limiting, and security events
- **Cache & Redis** - Hit rates, connection pools, and operation latency
- **Queues & Jobs** - Job throughput, queue depth, and cron task monitoring

---

## Available Dashboards

Armature provides four pre-built Grafana dashboards in `templates/grafana/`:

| Dashboard | File | Description |
|-----------|------|-------------|
| Application Overview | `armature-overview.json` | High-level health and performance metrics |
| Auth & Security | `armature-auth-security.json` | Authentication flows and security monitoring |
| Cache & Redis | `armature-cache-redis.json` | Caching effectiveness and Redis performance |
| Queues & Jobs | `armature-queues-jobs.json` | Background job processing and cron tasks |

---

## Quick Start

Get dashboards running in under 5 minutes:

### 1. Enable Metrics in Your App

Ensure your Armature application exposes Prometheus metrics:

```rust
// main.rs
use armature::prelude::*;
use armature_core::metrics::MetricsConfig;

#[tokio::main]
async fn main() {
    // Enable Prometheus metrics endpoint
    let metrics = MetricsConfig::builder()
        .enabled(true)
        .endpoint("/metrics")
        .include_default_labels(true)
        .build();

    let app = Application::builder()
        .metrics(metrics)
        .build();

    app.listen(3000).await.unwrap();
}

// Metrics are now available at GET /metrics
```

### 2. Configure Prometheus

Add your Armature app as a scrape target:

```yaml
# prometheus.yml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'armature'
    static_configs:
      - targets: ['your-app:3000']
    metrics_path: '/metrics'
    scrape_interval: 15s

  # For multiple instances
  - job_name: 'armature-prod'
    static_configs:
      - targets:
        - 'api-1:3000'
        - 'api-2:3000'
        - 'api-3:3000'
```

### 3. Import Dashboards

Import the JSON files into Grafana:

1. Open Grafana → **Dashboards** → **Import**
2. Upload the JSON file from `templates/grafana/`
3. Select your Prometheus data source
4. Click **Import**

---

## Application Overview Dashboard

The main dashboard for monitoring application health:

### Top Row Stats

- **Request Rate** — HTTP requests per second
- **P95 Latency** — 95th percentile response time (threshold: 100ms yellow, 500ms red)
- **Error Rate** — Percentage of 5xx responses (threshold: 1% yellow, 5% red)
- **Active Instances** — Number of running application instances
- **CPU Usage** — Average CPU utilization (threshold: 70% yellow, 90% red)
- **Memory Usage** — Average memory utilization (threshold: 70% yellow, 90% red)

### HTTP Metrics Panels

- **Request Rate by Method** — GET, POST, PUT, DELETE breakdown
- **Request Rate by Status** — 2xx (green), 4xx (yellow), 5xx (red)
- **Response Time Percentiles** — p50, p90, p95, p99 over time
- **Top 10 Endpoints** — Highest traffic endpoints

### System Resources Panels

- **CPU by Instance** — Per-instance CPU tracking
- **Memory by Instance** — Per-instance memory in bytes
- **Tokio Runtime Threads** — Worker and blocking thread counts
- **Active HTTP Connections** — Open connection count

---

## Auth & Security Dashboard

Monitor authentication and security events:

### Authentication Metrics

- Successful/Failed Logins (5-minute rolling)
- Active Sessions count
- JWT Tokens Issued rate
- JWT Validation Failures
- OAuth Callbacks by provider

### Rate Limiting

- Allowed vs rejected requests
- Top rate-limited endpoints

### Security Events

- Events by type
- Blocked requests by reason
- Top failed login IPs (24h table)

---

## Cache & Redis Dashboard

Track caching effectiveness and Redis performance:

### Cache Overview

- Cache Hit Rate (threshold: <90% yellow, <70% red)
- Cache Ops/sec
- Cache Entries count
- Cache Memory usage
- P95 Latency
- Evictions count

### Multi-Tier Caching

- L1 vs L2 hit rates
- Operations by type (GET/SET/DELETE)

### Redis Metrics

- Connection pool (active/idle)
- Command latency (p50/p95/p99)
- Top commands

---

## Queues & Jobs Dashboard

Monitor background job processing:

### Queue Overview

- Pending Jobs (threshold: 100 yellow, 500 red)
- Processing Jobs
- Throughput (jobs/sec)
- Failure Rate (threshold: 1% yellow, 5% red)
- Dead Letter Queue
- Active Workers

### Latency

- Processing duration by job type
- Wait time (queue latency)

### Scheduled Tasks

- Cron executions by task/status
- Task duration (P95)
- Retries by type
- Failures by error

---

## Docker Compose Setup

Full monitoring stack with Docker Compose:

```yaml
# docker-compose.monitoring.yml
version: '3.8'

services:
  prometheus:
    image: prom/prometheus:latest
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
      - prometheus_data:/prometheus
    ports:
      - "9090:9090"
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
      - '--storage.tsdb.retention.time=30d'

  grafana:
    image: grafana/grafana:latest
    volumes:
      - grafana_data:/var/lib/grafana
      - ./templates/grafana:/var/lib/grafana/dashboards/armature
      - ./grafana-provisioning:/etc/grafana/provisioning
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
    ports:
      - "3001:3000"
    depends_on:
      - prometheus

  your-app:
    build: .
    ports:
      - "3000:3000"
    environment:
      - METRICS_ENABLED=true

volumes:
  prometheus_data:
  grafana_data:
```

---

## Setting Up Alerts

Configure Grafana alerts for critical thresholds:

| Metric | Warning | Critical |
|--------|---------|----------|
| P95 Latency | > 100ms | > 500ms |
| Error Rate | > 1% | > 5% |
| CPU Usage | > 70% | > 90% |
| Memory Usage | > 70% | > 90% |
| Cache Hit Rate | < 90% | < 70% |
| Queue Depth | > 100 | > 500 |
| Job Failure Rate | > 1% | > 5% |
| Dead Letter Queue | > 10 | > 50 |

To create alerts in Grafana:

1. Edit the panel you want to alert on
2. Go to the **Alert** tab
3. Click **Create alert rule**
4. Configure thresholds and notification channels

---

## Metric Reference

These metric names are expected by the dashboards:

### HTTP Metrics

```text
http_requests_total{method, status, endpoint}
http_request_duration_seconds_bucket{le, endpoint}
http_connections_active
```

### Authentication Metrics

```text
auth_login_attempts_total{status}
auth_active_sessions
auth_jwt_tokens_issued_total
auth_jwt_validation_failures_total
auth_oauth_callbacks_total{provider}
ratelimit_requests_total{status, endpoint}
```

### Cache Metrics

```text
cache_hits_total{tier}
cache_misses_total{tier}
cache_operations_total{operation}
cache_entries_total
cache_memory_bytes
cache_evictions_total{reason}
```

### Queue Metrics

```text
queue_jobs_pending{queue}
queue_jobs_processing{queue}
queue_jobs_completed_total{queue}
queue_jobs_failed_total{queue, error_type}
queue_jobs_dead_letter{queue}
queue_workers_active
```

---

## Troubleshooting

### No Data Displayed

1. **Verify metrics endpoint** — `curl http://your-app:3000/metrics`
2. **Check Prometheus targets** — Status → Targets in Prometheus UI
3. **Verify job label** — Ensure `$job` variable matches your prometheus.yml
4. **Check data source** — Test connection in Grafana data sources

### Missing Metrics

Enable the features that expose the metrics you need:

```rust
// Enable all features that export metrics
let app = Application::builder()
    .metrics(MetricsConfig::default())      // HTTP metrics
    .cache(CacheConfig::default())          // Cache metrics
    .queue(QueueConfig::default())          // Queue metrics
    .auth(AuthConfig::default())            // Auth metrics
    .build();
```

---

## Related Documentation

- [Prometheus Metrics](metrics-guide.md) — Configure metrics export in your application
- [Health Checks](health-check-guide.md) — Kubernetes-ready health probes
- [OpenTelemetry](opentelemetry-guide.md) — Distributed tracing and observability

### See Also

- [Logging Guide](logging-guide.md)
- [Error Tracking](error-correlation-guide.md)
- [Docker Deployment](docker-guide.md)
