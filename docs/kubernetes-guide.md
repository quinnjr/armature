# Kubernetes Guide

This guide covers deploying and operating Armature applications on Kubernetes.

## Table of Contents

- [Overview](#overview)
- [Basic Deployment](#basic-deployment)
- [Service Configuration](#service-configuration)
- [Ingress with Ferron](#ingress-with-ferron)
- [ConfigMaps and Secrets](#configmaps-and-secrets)
- [Horizontal Pod Autoscaler](#horizontal-pod-autoscaler)
- [Health Probes](#health-probes)
- [Resource Management](#resource-management)
- [Best Practices](#best-practices)

## Overview

Kubernetes provides container orchestration for Armature applications with:

- **Automatic scaling** based on CPU/memory or custom metrics
- **Self-healing** with health probes and automatic restarts
- **Rolling updates** for zero-downtime deployments
- **Service discovery** with DNS-based service routing
- **Load balancing** across pod replicas

## Basic Deployment

### Deployment Manifest

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: armature-api
  labels:
    app: armature-api
spec:
  replicas: 3
  selector:
    matchLabels:
      app: armature-api
  template:
    metadata:
      labels:
        app: armature-api
    spec:
      containers:
        - name: api
          image: your-registry/armature-api:latest
          ports:
            - containerPort: 3000
          env:
            - name: RUST_LOG
              value: "info"
            - name: DATABASE_URL
              valueFrom:
                secretKeyRef:
                  name: api-secrets
                  key: database-url
          resources:
            limits:
              cpu: "1"
              memory: "512Mi"
            requests:
              cpu: "250m"
              memory: "256Mi"
          livenessProbe:
            httpGet:
              path: /live
              port: 3000
            initialDelaySeconds: 10
            periodSeconds: 10
          readinessProbe:
            httpGet:
              path: /ready
              port: 3000
            initialDelaySeconds: 5
            periodSeconds: 5
          startupProbe:
            httpGet:
              path: /health
              port: 3000
            failureThreshold: 30
            periodSeconds: 10
```

## Service Configuration

```yaml
apiVersion: v1
kind: Service
metadata:
  name: armature-api
spec:
  selector:
    app: armature-api
  ports:
    - port: 80
      targetPort: 3000
  type: ClusterIP
```

## Ingress with Ferron

### Ferron ConfigMap

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: ferron-config
data:
  ferron.conf: |
    api.example.com {
        tls auto
        hsts max_age=31536000
        gzip level=6

        header "X-Frame-Options" "DENY"
        header "X-Content-Type-Options" "nosniff"

        lb_method "round_robin"
        proxy "http://armature-api:80"

        lb_health_check interval=10 path="/health" threshold=3
    }
```

### Ferron Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: ferron
spec:
  replicas: 2
  selector:
    matchLabels:
      app: ferron
  template:
    metadata:
      labels:
        app: ferron
    spec:
      containers:
        - name: ferron
          image: ferronweb/ferron:latest
          ports:
            - containerPort: 80
            - containerPort: 443
          volumeMounts:
            - name: config
              mountPath: /etc/ferron
            - name: certs
              mountPath: /var/lib/ferron/certs
          resources:
            limits:
              cpu: "500m"
              memory: "256Mi"
            requests:
              cpu: "100m"
              memory: "128Mi"
      volumes:
        - name: config
          configMap:
            name: ferron-config
        - name: certs
          persistentVolumeClaim:
            claimName: ferron-certs
---
apiVersion: v1
kind: Service
metadata:
  name: ferron
spec:
  type: LoadBalancer
  selector:
    app: ferron
  ports:
    - name: http
      port: 80
      targetPort: 80
    - name: https
      port: 443
      targetPort: 443
```

## ConfigMaps and Secrets

### ConfigMap for Application Config

```yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: api-config
data:
  LOG_LEVEL: "info"
  CACHE_TTL: "3600"
  RATE_LIMIT: "100"
```

### Secrets for Sensitive Data

```yaml
apiVersion: v1
kind: Secret
metadata:
  name: api-secrets
type: Opaque
stringData:
  database-url: "postgres://user:pass@db:5432/app"
  jwt-secret: "your-secret-key"
  redis-url: "redis://redis:6379"
```

### Using in Deployment

```yaml
env:
  - name: LOG_LEVEL
    valueFrom:
      configMapKeyRef:
        name: api-config
        key: LOG_LEVEL
  - name: DATABASE_URL
    valueFrom:
      secretKeyRef:
        name: api-secrets
        key: database-url
```

## Horizontal Pod Autoscaler

### CPU-Based Scaling

```yaml
apiVersion: autoscaling/v2
kind: HorizontalPodAutoscaler
metadata:
  name: armature-api-hpa
spec:
  scaleTargetRef:
    apiVersion: apps/v1
    kind: Deployment
    name: armature-api
  minReplicas: 3
  maxReplicas: 20
  metrics:
    - type: Resource
      resource:
        name: cpu
        target:
          type: Utilization
          averageUtilization: 70
    - type: Resource
      resource:
        name: memory
        target:
          type: Utilization
          averageUtilization: 80
  behavior:
    scaleUp:
      stabilizationWindowSeconds: 60
      policies:
        - type: Pods
          value: 4
          periodSeconds: 60
    scaleDown:
      stabilizationWindowSeconds: 300
      policies:
        - type: Percent
          value: 10
          periodSeconds: 60
```

## Health Probes

### Implementing Probes in Armature

```rust
#[controller("")]
#[derive(Default, Clone)]
struct HealthController;

#[routes]
impl HealthController {
    // Liveness probe - Is the app running?
    #[get("/live")]
    async fn live() -> Result<HttpResponse, Error> {
        HttpResponse::ok().with_body(b"OK".to_vec())
    }

    // Readiness probe - Is the app ready for traffic?
    #[get("/ready")]
    async fn ready() -> Result<HttpResponse, Error> {
        // Check dependencies
        let db_ok = check_database().await;
        let cache_ok = check_cache().await;

        if db_ok && cache_ok {
            HttpResponse::ok().with_body(b"READY".to_vec())
        } else {
            Err(Error::internal("Not ready"))
        }
    }

    // Startup probe - Has the app started?
    #[get("/health")]
    async fn health() -> Result<HttpResponse, Error> {
        HttpResponse::json(&serde_json::json!({
            "status": "healthy",
            "version": env!("CARGO_PKG_VERSION")
        }))
    }
}
```

## Resource Management

### Pod Disruption Budget

```yaml
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: armature-api-pdb
spec:
  minAvailable: 2
  selector:
    matchLabels:
      app: armature-api
```

### Resource Quotas

```yaml
apiVersion: v1
kind: ResourceQuota
metadata:
  name: api-quota
spec:
  hard:
    requests.cpu: "4"
    requests.memory: "4Gi"
    limits.cpu: "8"
    limits.memory: "8Gi"
    pods: "20"
```

### Limit Ranges

```yaml
apiVersion: v1
kind: LimitRange
metadata:
  name: api-limits
spec:
  limits:
    - default:
        cpu: "500m"
        memory: "256Mi"
      defaultRequest:
        cpu: "100m"
        memory: "128Mi"
      max:
        cpu: "2"
        memory: "1Gi"
      min:
        cpu: "50m"
        memory: "64Mi"
      type: Container
```

## Best Practices

### 1. Use Namespaces

```yaml
apiVersion: v1
kind: Namespace
metadata:
  name: armature-production
```

### 2. Set Pod Anti-Affinity

```yaml
spec:
  affinity:
    podAntiAffinity:
      preferredDuringSchedulingIgnoredDuringExecution:
        - weight: 100
          podAffinityTerm:
            labelSelector:
              matchLabels:
                app: armature-api
            topologyKey: kubernetes.io/hostname
```

### 3. Use Rolling Updates

```yaml
spec:
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 25%
      maxUnavailable: 25%
```

### 4. Configure Graceful Shutdown

```rust
use armature_framework::shutdown::GracefulShutdown;
use tokio::signal;

async fn shutdown_signal() {
    signal::unix::signal(signal::unix::SignalKind::terminate())
        .expect("failed to create SIGTERM handler")
        .recv()
        .await;
}
```

```yaml
spec:
  terminationGracePeriodSeconds: 60
```

### 5. Use Service Accounts

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: armature-api
---
spec:
  serviceAccountName: armature-api
```

## Summary

Key Kubernetes deployment considerations:

1. **Configure health probes** - liveness, readiness, startup
2. **Set resource limits** - prevent resource contention
3. **Use HPA** - automatic scaling based on metrics
4. **Configure PDB** - maintain availability during disruptions
5. **Use secrets** - never hardcode sensitive data
6. **Enable rolling updates** - zero-downtime deployments
7. **Use Ferron** - high-performance ingress with TLS

