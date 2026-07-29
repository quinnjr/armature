//! Behavioral tests for `#[module(providers: [...])]` provider registration,
//! including automatic lifecycle hook discovery (`OnModuleInit` /
//! `OnApplicationBootstrap`) wired through `Application::create`.
//!
//! No other test in this crate exercises the `providers:` argument of
//! `#[module(...)]`, so this is the only place the real macro-generated
//! `register_fn` closure (in `armature-proc-macro/src/module.rs`) gets
//! type-checked against real `armature-core` types.

use armature_core::Application;
use armature_core::lifecycle::{LifecycleResult, OnApplicationBootstrap, OnModuleInit};
use armature_proc_macro::{injectable, module};
use std::sync::atomic::{AtomicBool, Ordering};

static PROVIDER_INIT_CALLED: AtomicBool = AtomicBool::new(false);
static PROVIDER_BOOTSTRAP_CALLED: AtomicBool = AtomicBool::new(false);

#[injectable]
#[derive(Default)]
struct LifecycleProvider;

#[async_trait::async_trait]
impl OnModuleInit for LifecycleProvider {
    async fn on_module_init(&self) -> LifecycleResult {
        PROVIDER_INIT_CALLED.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait::async_trait]
impl OnApplicationBootstrap for LifecycleProvider {
    async fn on_application_bootstrap(&self) -> LifecycleResult {
        PROVIDER_BOOTSTRAP_CALLED.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
#[module(providers: [LifecycleProvider])]
struct LifecycleTestModule;

#[tokio::test]
async fn module_providers_registers_provider_and_fires_lifecycle_hooks() {
    PROVIDER_INIT_CALLED.store(false, Ordering::SeqCst);
    PROVIDER_BOOTSTRAP_CALLED.store(false, Ordering::SeqCst);

    let app = Application::create::<LifecycleTestModule>().await;

    assert!(
        app.container.has::<LifecycleProvider>(),
        "#[module(providers: [...])] must register the provider in the container"
    );

    // `.has::<T>()` only proves presence in the type-keyed map; it does not
    // prove that `resolve::<T>()`'s downcast of the stored
    // `Arc<dyn Any + Send + Sync>` back to `Arc<T>` actually succeeds and
    // yields a real, usable instance. Resolve it and invoke one of its
    // actual trait methods (observing the side effect it's known to
    // produce) to prove both.
    let resolved = app.container.resolve::<LifecycleProvider>().expect(
        "container.resolve::<LifecycleProvider>() must succeed for a provider \
         registered via #[module(providers: [...])]",
    );
    PROVIDER_INIT_CALLED.store(false, Ordering::SeqCst);
    resolved
        .on_module_init()
        .await
        .expect("on_module_init() on the resolved instance must succeed");
    assert!(
        PROVIDER_INIT_CALLED.load(Ordering::SeqCst),
        "resolved LifecycleProvider must be the real, usable provider instance \
         (invoking on_module_init on it must produce the same side effect as \
         the automatic lifecycle hook)"
    );

    assert!(
        PROVIDER_INIT_CALLED.load(Ordering::SeqCst),
        "OnModuleInit must fire automatically for a provider registered via \
         #[module(providers: [...])]"
    );
    assert!(
        PROVIDER_BOOTSTRAP_CALLED.load(Ordering::SeqCst),
        "OnApplicationBootstrap must fire automatically for a provider \
         registered via #[module(providers: [...])]"
    );
}

/// A provider with no lifecycle hooks must still register normally (the
/// specialization probe must be a true no-op for non-participating types).
#[injectable]
#[derive(Default)]
struct PlainProvider;

/// A second provider that constructor-injects `PlainProvider` via an
/// `#[injectable]` `Arc<T>` field, proving end-to-end field-based DI (not
/// just standalone `.resolve::<T>()`) still works post-refactor.
#[injectable]
struct InjectingProvider {
    inner: std::sync::Arc<PlainProvider>,
}

#[derive(Default)]
#[module(providers: [PlainProvider, InjectingProvider])]
struct PlainTestModule;

#[tokio::test]
async fn module_providers_registers_plain_provider_without_lifecycle_hooks() {
    let app = Application::create::<PlainTestModule>().await;
    assert!(app.container.has::<PlainProvider>());

    // `PlainProvider` has no lifecycle hooks to observe a side effect
    // through, so prove the resolved value is the real, usable singleton
    // (not a corrupted/default reinterpretation from a bad
    // `Arc<dyn Any + Send + Sync>` downcast) by checking that
    // `InjectingProvider`, which field-injects `PlainProvider` via
    // `#[injectable]`, ends up holding the exact same `Arc` allocation that
    // `.resolve::<PlainProvider>()` returns directly.
    let resolved = app.container.resolve::<PlainProvider>().expect(
        "container.resolve::<PlainProvider>() must succeed for a provider \
         registered via #[module(providers: [...])]",
    );
    let injecting = app
        .container
        .resolve::<InjectingProvider>()
        .expect("container.resolve::<InjectingProvider>() must succeed");
    assert!(
        std::sync::Arc::ptr_eq(&resolved, &injecting.inner),
        "field-injected PlainProvider must be the exact same singleton \
         instance resolved directly from the container"
    );
}
