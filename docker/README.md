# Armature Docker Compose Examples

Production-ready Docker Compose configurations for deploying Armature applications.

## Available Configurations

### 1. Development (`development/`)

Lightweight stack for local development:

```bash
cd docker/development
docker compose up -d
```

**Services:**
| Service | Port | Description |
|---------|------|-------------|
| PostgreSQL | 5432 | Database |
| Redis | 6379 | Cache/sessions |
| Adminer | 8080 | Database UI |
| MailHog | 8025 | Email testing |
| RedisInsight | 8001 | Redis GUI |
| LocalStack | 4566 | AWS emulation |
| MinIO | 9000/9001 | S3 storage |

### 2. Observability (`observability/`)

Complete monitoring stack with Prometheus, Grafana, and distributed tracing:

```bash
cd docker/observability
docker compose up -d
```

**Services:**
| Service | Port | Description |
|---------|------|-------------|
| Armature API | 3000 | Your application |
| Prometheus | 9090 | Metrics collection |
| Grafana | 3001 | Dashboards (admin/admin) |
| Jaeger | 16686 | Distributed tracing |
| Loki | 3100 | Log aggregation |
| Alertmanager | 9093 | Alert routing |
| Node Exporter | 9101 | Host metrics |
| cAdvisor | 8080 | Container metrics |

### 3. Full Stack (`full-stack/`)

Production-ready setup with load balancing, database replication, and message queues:

```bash
cd docker/full-stack
docker compose up -d
```

**Services:**
| Service | Port | Description |
|---------|------|-------------|
| Ferron | 80/443 | Load balancer (Rust-native) |
| API (x2) | - | Application instances |
| PostgreSQL Primary | 5432 | Primary database |
| PostgreSQL Replica | - | Read replica |
| Redis | 6379 | Cache |
| RabbitMQ | 5672/15672 | Message queue |
| Elasticsearch | 9200 | Search |
| MinIO | 9000/9001 | Object storage |

### 4. Microservices (`microservices/`)

Multi-service architecture with API gateway:

```bash
cd docker/microservices
docker compose up -d
```

**Services:**
| Service | Port | Description |
|---------|------|-------------|
| Kong Gateway | 8000/8001 | API Gateway |
| User Service | - | Authentication |
| Product Service | - | Product catalog |
| Order Service | - | Orders & cart |
| Notification Service | - | Email/push |
| Worker Service | - | Background jobs |
| Jaeger | 16686 | Tracing |

## Quick Start

### Prerequisites

- Docker 24+
- Docker Compose v2

### Start Development Environment

```bash
# Clone the repository
git clone https://github.com/your-org/armature
cd armature/docker/development

# Start services
docker compose up -d

# Check status
docker compose ps

# View logs
docker compose logs -f

# Stop services
docker compose down
```

### Environment Variables

Create a `.env` file for secrets:

```env
# Required
JWT_SECRET=your-super-secret-jwt-key-change-in-production

# Optional
RUST_LOG=info,armature=debug
```

### Connecting Your Application

Add these environment variables to your Armature app:

```env
DATABASE_URL=postgres://armature:armature@localhost:5432/armature_dev
REDIS_URL=redis://localhost:6379
SMTP_HOST=localhost
SMTP_PORT=1025
```

## Architecture Diagrams

### Development Stack

```
┌─────────────────────────────────────────────┐
│              Your Application               │
│        cargo run --example your_app         │
└─────────────────────────────────────────────┘
            │         │         │
            ▼         ▼         ▼
┌─────────┐ ┌───────┐ ┌─────────┐
│PostgreSQL│ │ Redis │ │ MailHog │
│  :5432   │ │ :6379 │ │  :8025  │
└─────────┘ └───────┘ └─────────┘
```

### Production Stack

```
                    ┌──────────┐
                    │  Client  │
                    └────┬─────┘
                         │
                    ┌────▼─────┐
                    │  Ferron  │
                    │   :80    │
                    └────┬─────┘
                    ┌────┴────┐
               ┌────▼───┐ ┌───▼────┐
               │ API-1  │ │ API-2  │
               └────┬───┘ └───┬────┘
                    │         │
    ┌───────────────┼─────────┼───────────────┐
    │               │         │               │
┌───▼───┐     ┌────▼────┐ ┌──▼───┐     ┌─────▼─────┐
│Postgres│     │  Redis  │ │RabbitMQ│   │Elasticsearch│
│ :5432  │     │  :6379  │ │ :5672 │   │   :9200    │
└────────┘     └─────────┘ └───────┘   └───────────┘
```

### Microservices

```
┌─────────────────────────────────────────────────────────┐
│                      Kong Gateway                        │
│                        :8000                             │
└───┬─────────────┬─────────────┬─────────────┬───────────┘
    │             │             │             │
┌───▼───┐    ┌───▼───┐    ┌───▼───┐    ┌───▼───┐
│ Users │    │Products│    │Orders │    │Notify │
└───┬───┘    └───┬───┘    └───┬───┘    └───┬───┘
    │             │             │             │
    └──────┬──────┴──────┬──────┴──────┬──────┘
           │             │             │
     ┌─────▼─────┐ ┌─────▼─────┐ ┌─────▼─────┐
     │ PostgreSQL│ │   Redis   │ │ RabbitMQ  │
     └───────────┘ └───────────┘ └───────────┘
```

## Customization

### Adding Services

1. Edit the appropriate `docker-compose.yml`
2. Add your service definition
3. Update networks and depends_on
4. Add volumes if needed

### Scaling Services

```bash
# Scale API to 4 instances
docker compose up -d --scale api=4
```

### Resource Limits

Modify `deploy.resources` in compose files:

```yaml
deploy:
  resources:
    limits:
      cpus: "1"
      memory: 512M
    reservations:
      cpus: "0.25"
      memory: 128M
```

## Monitoring & Debugging

### View Logs

```bash
# All services
docker compose logs -f

# Specific service
docker compose logs -f api

# Last 100 lines
docker compose logs --tail=100 api
```

### Shell Access

```bash
# Enter container
docker compose exec api sh

# Run command
docker compose exec postgres psql -U armature -d armature_dev
```

### Health Checks

```bash
# Check service health
docker compose ps

# Inspect health
docker inspect --format='{{.State.Health.Status}}' container_name
```

## Troubleshooting

### Port Conflicts

If a port is already in use:

```bash
# Find process using port
lsof -i :5432

# Or change port in compose file
ports:
  - "5433:5432"  # Use 5433 on host
```

### Volume Permissions

```bash
# Fix permissions on Linux
sudo chown -R 1000:1000 ./data
```

### Database Connection Issues

```bash
# Wait for PostgreSQL
docker compose exec postgres pg_isready -U armature

# Reset database
docker compose down -v
docker compose up -d
```

## Production Checklist

- [ ] Change default passwords in `.env`
- [ ] Enable TLS/SSL
- [ ] Configure proper log rotation
- [ ] Set up backup procedures
- [ ] Configure alerting
- [ ] Review resource limits
- [ ] Enable health checks
- [ ] Set up monitoring

## License

MIT License - See [LICENSE](../LICENSE)

