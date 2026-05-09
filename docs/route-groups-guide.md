# Route Groups Guide

Comprehensive guide to organizing routes with Route Groups in Armature.

## Table of Contents

- [Overview](#overview)
- [Features](#features)
- [Basic Usage](#basic-usage)
- [Shared Configuration](#shared-configuration)
- [Nested Groups](#nested-groups)
- [Best Practices](#best-practices)
- [API Reference](#api-reference)
- [Examples](#examples)
- [Summary](#summary)

---

## Overview

Route Groups allow you to organize routes with shared configuration, making your routing code more maintainable and DRY (Don't Repeat Yourself).

Route groups provide:
- **Path prefixes** - Automatic prefix for all routes in the group
- **Shared middleware** - Apply middleware to all routes in the group
- **Shared guards** - Apply authorization to all routes in the group
- **Nested configuration** - Groups can inherit from parent groups

---

## Features

- ✅ Path prefix inheritance
- ✅ Shared middleware application
- ✅ Shared guard application
- ✅ Nested groups with configuration merging
- ✅ Fluent builder API
- ✅ Type-safe configuration

---

## Basic Usage

### Creating a Route Group

```rust
use armature_core::*;

// Create a basic API group with prefix
let api_group = RouteGroup::new()
    .prefix("/api/v1");

// All routes in this group will have /api/v1 prefix
let user_route = api_group.apply_prefix("/users");
// Result: "/api/v1/users"
```

### With Middleware

```rust
use armature_core::*;
use std::sync::Arc;

let api_group = RouteGroup::new()
    .prefix("/api/v1")
    .middleware(Arc::new(LoggerMiddleware))
    .middleware(Arc::new(CorsMiddleware::default()));

// All routes in this group will have logging and CORS enabled
```

### With Guards

```rust
use armature_core::*;

let protected_group = RouteGroup::new()
    .prefix("/api/v1/admin")
    .guard(Box::new(AuthenticationGuard))
    .guard(Box::new(RolesGuard::new(vec!["admin".to_string()])));

// All routes require authentication AND admin role
```

---

## Shared Configuration

### Path Prefixes

Route groups automatically prepend prefixes to all routes:

```rust
use armature_core::*;

let api = RouteGroup::new().prefix("/api/v1");

// Apply prefix to routes
assert_eq!(api.apply_prefix("/users"), "/api/v1/users");
assert_eq!(api.apply_prefix("/posts"), "/api/v1/posts");
assert_eq!(api.apply_prefix("/comments"), "/api/v1/comments");
```

### Multiple Middleware

Middleware are applied in the order they're added:

```rust
let group = RouteGroup::new()
    .middleware(Arc::new(LoggerMiddleware))
    .middleware(Arc::new(CorsMiddleware::default()))
    .middleware(Arc::new(CompressionMiddleware::new()));

// Execution order: Logger → CORS → Compression → Handler
```

### Multiple Guards

All guards must pass for access to be granted (AND logic):

```rust
let group = RouteGroup::new()
    .guard(Box::new(AuthenticationGuard))
    .guard(Box::new(RolesGuard::new(vec!["admin".to_string()])))
    .guard(Box::new(ApiKeyGuard::new(vec!["key123".to_string()])));

// Request must pass ALL guards
```

---

## Nested Groups

Groups can inherit configuration from parent groups:

### Basic Nesting

```rust
use armature_core::*;

let api = RouteGroup::new()
    .prefix("/api")
    .middleware(Arc::new(LoggerMiddleware));

let v1 = RouteGroup::new()
    .prefix("/v1")
    .with_parent(&api);

// v1 inherits:
// - Prefix: "/api/v1"
// - Middleware: LoggerMiddleware

let admin = RouteGroup::new()
    .prefix("/admin")
    .guard(Box::new(AdminGuard))
    .with_parent(&v1);

// admin inherits:
// - Prefix: "/api/v1/admin"
// - Middleware: LoggerMiddleware
// - Guard: AdminGuard
```

### Configuration Merging Rules

When using `with_parent()`:

1. **Prefixes are concatenated** - parent prefix + child prefix
2. **Middleware are combined** - parent middleware execute first
3. **Guards are from child only** - cannot clone Box<dyn Guard>

---

## Best Practices

### 1. Organize by API Version

```rust
let v1 = RouteGroup::new()
    .prefix("/api/v1")
    .middleware(Arc::new(LoggerMiddleware));

let v2 = RouteGroup::new()
    .prefix("/api/v2")
    .middleware(Arc::new(LoggerMiddleware))
    .middleware(Arc::new(RateLimitMiddleware::new()));
```

### 2. Group by Authentication Level

```rust
let public = RouteGroup::new()
    .prefix("/api/public");

let authenticated = RouteGroup::new()
    .prefix("/api/auth")
    .guard(Box::new(AuthenticationGuard));

let admin = RouteGroup::new()
    .prefix("/api/admin")
    .guard(Box::new(AuthenticationGuard))
    .guard(Box::new(AdminGuard));
```

### 3. Combine Strategies

```rust
// Base API group
let api = RouteGroup::new()
    .prefix("/api")
    .middleware(Arc::new(LoggerMiddleware));

// Version groups
let v1 = RouteGroup::new()
    .prefix("/v1")
    .with_parent(&api);

// Resource groups within v1
let v1_users = RouteGroup::new()
    .prefix("/users")
    .guard(Box::new(AuthenticationGuard))
    .with_parent(&v1);

let v1_admin = RouteGroup::new()
    .prefix("/admin")
    .guard(Box::new(AuthenticationGuard))
    .guard(Box::new(AdminGuard))
    .with_parent(&v1);
```

---

## API Reference

### RouteGroup Methods

| Method | Description |
|--------|-------------|
| `prefix(path)` | Set the path prefix for this group |
| `middleware(mw)` | Add a single middleware |
| `with_middleware(mws)` | Add multiple middleware |
| `guard(guard)` | Add a single guard |
| `with_guards(guards)` | Add multiple guards |
| `get_prefix()` | Get the current prefix |
| `apply_prefix(path)` | Apply prefix to a path |
| `get_middleware()` | Get all middleware |
| `get_guards()` | Get all guards |
| `with_parent(parent)` | Inherit from parent group |

---

## Summary

**Key Points:**

1. **RouteGroup organizes routes** with shared configuration
2. **Prefixes are automatically applied** and normalized
3. **Middleware stack in order** they're added
4. **All guards must pass** (AND logic)
5. **Nest groups with `with_parent()`** for inheritance
6. **Use for API versions, auth levels, resources**

**Quick Reference:**

```rust
// Basic group
let group = RouteGroup::new()
    .prefix("/api/v1")
    .middleware(Arc::new(LoggerMiddleware))
    .guard(Box::new(AuthenticationGuard));

// Nested group
let child = RouteGroup::new()
    .prefix("/admin")
    .guard(Box::new(AdminGuard))
    .with_parent(&group);

// Apply prefix
let path = child.apply_prefix("/users");
// Result: "/api/v1/admin/users"
```

**Benefits:**

- 📦 **DRY** - Don't repeat middleware/guards
- 🎯 **Organized** - Clear route structure
- 🔒 **Secure** - Consistent auth application
- 📈 **Scalable** - Easy to add new groups
- 🔧 **Maintainable** - Change once, apply everywhere

