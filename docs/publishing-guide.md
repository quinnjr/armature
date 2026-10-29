# Publishing Guide

Guide for publishing Armature crates to crates.io.

## Table of Contents

- [Overview](#overview)
- [Prerequisites](#prerequisites)
- [Publishing Scripts](#publishing-scripts)
- [Publish Order](#publish-order)
- [Step-by-Step Publishing](#step-by-step-publishing)
- [Troubleshooting](#troubleshooting)

---

## Overview

The Armature workspace contains 47+ crates with inter-dependencies. Publishing to crates.io requires:

1. **Correct Order**: Crates must be published in dependency order
2. **Required Metadata**: All crates need license, description, and other metadata

Sibling crates already depend on each other by version (for example
`armature-core = { version = "0.12" }`); the workspace root's `[patch.crates-io]`
table points those requirements at the local checkouts during development, so
nothing has to be rewritten before publishing.

`scripts/publish.sh` publishes every crate in dependency order.

Publishing is **manual only**: it runs from a maintainer's machine, never from
CI. Keeping the registry token out of CI means no third-party action, cached
build directory, or dependency build script executed in CI can ever read it.

---

## Prerequisites

### 1. crates.io Account

Create an account at [crates.io](https://crates.io) and log in:

```bash
cargo login
```

Use a token scoped to `publish-update` for the `armature-*` crates only, with
an expiry. Never store it in a repository secret.

### 2. Required Metadata

Each crate's `Cargo.toml` must include:

```toml
[package]
name = "armature-xxx"
version = "0.1.0"
license = "Apache-2.0"
description = "Description of this crate"
repository = "https://github.com/quinnjr/armature"
```

### 3. Check Readiness

```bash
./scripts/publish.sh --check
```

Output shows which crates are ready and which need fixes:

```
✓ armature-log
✓ armature-core
! armature-auth: Missing 'license' field
```

---

## Publishing Scripts

### publish.sh

Main publishing script that handles dependency order.

```bash
# Show publish order (dry run)
# Also shows which versions are already on crates.io
./scripts/publish.sh --dry-run

# Check all crates are ready
./scripts/publish.sh --check

# Publish all crates
# Automatically skips versions already published on crates.io
./scripts/publish.sh

# Publish single crate
./scripts/publish.sh --single armature-log

# Resume from a specific crate
./scripts/publish.sh --from armature-auth

# Skip problematic crates
./scripts/publish.sh --skip armature-cli --skip armature-ferron

# Force republish even if version exists on crates.io
./scripts/publish.sh --force

# Skip verification (faster, less safe)
./scripts/publish.sh --no-verify
```

---

## Publish Order

The script automatically computes the correct publish order using topological sort. Crates with no workspace dependencies are published first.

Example order (abbreviated):

```
1. armature-log           # No deps
2. armature-macros-utils  # No deps
3. armature-core          # Depends on armature-log
4. armature-jwt           # Depends on armature-core
5. armature-auth          # Depends on armature-jwt, armature-core
6. armature-cache         # Depends on armature-core, armature-redis
...
```

View the full order with version status:

```bash
./scripts/publish.sh --dry-run
```

Output shows crates.io status:

```
1. armature-log (v0.1.0) [on crates.io]     # Already published
2. armature-core (v0.1.0) [new version]     # New version to publish
3. armature-auth (v0.1.0) [new crate]       # First time publishing
```

---

## Version Verification

The publish script automatically checks crates.io before publishing:

- **Already published versions are skipped** (no errors)
- **New versions are published** normally
- **New crates** (never published) are published normally

### Behavior

```bash
./scripts/publish.sh
```

Output:
```
[INFO] Publishing armature-log v0.1.0...
[WARN] armature-log v0.1.0 is already published on crates.io (use --force to republish)
[INFO] Publishing armature-core v0.1.1...
[SUCCESS] armature-core v0.1.1 published successfully

Publishing complete!
  Published: 1
  Already on crates.io: 1
  Skipped: 0
```

### Force Republish

To attempt publishing even if the version exists:

```bash
./scripts/publish.sh --force
```

Note: crates.io will still reject duplicate versions. Use `--force` when:
- You need to retry a failed publish
- The API check returned an error

---

## Step-by-Step Publishing

### 1. Check Current State

```bash
./scripts/publish.sh --check
```

Fix any issues reported (missing metadata, etc.).

### 2. Verify Against the Lockfile

```bash
cargo metadata --locked --format-version 1 > /dev/null
cargo deny --workspace --all-features check
```

Publish from a clean checkout of the release tag with the committed
`Cargo.lock`, so what gets verified is what was reviewed.

### 3. Publish

```bash
# Dry run first
./scripts/publish.sh --dry-run

# Then publish
./scripts/publish.sh
```

The script:
- Publishes crates in dependency order
- Waits 60 seconds between publishes (configurable with `--delay`) for crates.io indexing
- Shows progress

---

## Troubleshooting

### "Rate limit exceeded"

crates.io has rate limits. The script waits 60 seconds between publishes, but you may need to wait longer:

```bash
./scripts/publish.sh --delay 120
```

### "Dependency not found"

The dependency hasn't been indexed yet. Wait a minute and retry:

```bash
./scripts/publish.sh --from armature-xxx
```

### "Missing required field"

Add the missing field to the crate's `Cargo.toml`:

```toml
[package]
license = "Apache-2.0"
description = "Your description"
```

### "Package already exists"

The version is already published. The script now automatically detects this and skips the crate:

```
[WARN] armature-xxx v0.1.0 is already published on crates.io
```

To publish a new version, bump the version in `Cargo.toml`:

```toml
version = "0.1.1"
```

### Partial Publish Failed

Resume from where it stopped:

```bash
./scripts/publish.sh --from armature-xxx
```

Or skip the problematic crate:

```bash
./scripts/publish.sh --skip armature-xxx
```

---

## Best Practices

### 1. Always Dry Run First

```bash
./scripts/publish.sh --dry-run
```

### 2. Publish Minor Updates Frequently

Small, frequent releases are easier to manage than large batches.

### 3. Use Semantic Versioning

- `0.1.0` → `0.1.1`: Bug fixes
- `0.1.0` → `0.2.0`: New features
- `0.1.0` → `1.0.0`: Breaking changes

### 4. Test Before Publishing

```bash
cargo test --all
cargo clippy --all
```

---

## Summary

### Quick Reference

```bash
# Check readiness
./scripts/publish.sh --check

# Publish
./scripts/publish.sh
```

### Environment

```bash
cargo login   # interactive; keeps the token out of shell history and CI
```

---

**Happy publishing!** 📦

