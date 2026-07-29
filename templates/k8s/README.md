# Kubernetes Manifests

Kubernetes templates, with production-oriented defaults (non-root, resource limits, health probes), for deploying Armature applications.

## Quick Start

```bash
# Apply all manifests
kubectl apply -k .

# Or apply individually
kubectl apply -f namespace.yaml
kubectl apply -f deployment.yaml
kubectl apply -f service.yaml
```

## Files

| File | Description |
|------|-------------|
| `namespace.yaml` | Kubernetes namespace |
| `deployment.yaml` | Main application deployment |
| `service.yaml` | ClusterIP service |
| `ingress.yaml` | Ingress with TLS |
| `hpa.yaml` | Horizontal Pod Autoscaler |
| `pdb.yaml` | Pod Disruption Budget |
| `configmap.yaml` | Application configuration |
| `secret.yaml` | Sensitive data (template only) |
| `serviceaccount.yaml` | Service account |
| `networkpolicy.yaml` | Network security policies |
| `kustomization.yaml` | Kustomize base |

## Customization

These manifests are plain, template-free YAML with sensible static defaults (app name `armature-api`, namespace `armature`, container port `3000`, image `ghcr.io/armature-rs/armature-api:latest`). They are syntactically validated in CI and can be applied directly with `kubectl apply -k .`, or customized with:

- **Kustomize**: Use overlays to patch values (see below) — this is the recommended approach
- **Helm**: See `../helm/armature/` for a fully parameterized Helm chart instead
- **Manual edits**: Just edit the YAML directly for one-off changes

### Using Kustomize

Create an overlay:

```yaml
# overlays/production/kustomization.yaml
apiVersion: kustomize.config.k8s.io/v1beta1
kind: Kustomization

resources:
  - ../../k8s

namespace: production

images:
  - name: ghcr.io/armature-rs/armature-api
    newTag: v1.0.0

patches:
  - patch: |-
      - op: replace
        path: /spec/replicas
        value: 5
    target:
      kind: Deployment
      name: armature-api
```

```bash
kubectl apply -k overlays/production
```

## Security Features

- ✅ Non-root user (UID 1000)
- ✅ Read-only root filesystem
- ✅ Dropped capabilities
- ✅ Network policies (optional)
- ✅ Service account without auto-mount
- ✅ Resource limits

## Health Checks

All deployments include:

- **Liveness probe**: `/health/live`
- **Readiness probe**: `/health/ready`
- **Startup probe**: `/health/live` (for slow starts)

## Monitoring

Deployments are annotated for Prometheus scraping:

```yaml
prometheus.io/scrape: "true"
prometheus.io/port: "3000"
prometheus.io/path: "/metrics"
```

