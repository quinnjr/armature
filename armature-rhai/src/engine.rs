//! Rhai engine configuration and management.

use crate::bindings::register_armature_api;
use crate::error::{Result, RhaiError};
use crate::script::{CompiledScript, ScriptCache, ScriptLoader};
use parking_lot::RwLock;
use rhai::{AST, Dynamic, Engine, Scope};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Rhai engine with Armature bindings.
///
/// Provides script compilation, caching, and execution.
pub struct RhaiEngine {
    /// The underlying Rhai engine.
    engine: Engine,
    /// Script cache for compiled scripts.
    cache: Arc<RwLock<ScriptCache>>,
    /// Script loader for file operations.
    loader: ScriptLoader,
    /// Configuration.
    config: EngineConfig,
}

/// Engine configuration.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Base directory for scripts.
    pub scripts_dir: PathBuf,
    /// Maximum operations per script execution.
    pub max_operations: Option<u64>,
    /// Maximum call stack depth.
    pub max_call_depth: usize,
    /// Maximum string length.
    pub max_string_size: usize,
    /// Maximum array size.
    pub max_array_size: usize,
    /// Maximum map size.
    pub max_map_size: usize,
    /// Enable hot reload (development mode).
    pub hot_reload: bool,
    /// Script file extension.
    pub extension: String,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            scripts_dir: PathBuf::from("./scripts"),
            max_operations: Some(100_000),
            max_call_depth: 64,
            max_string_size: 1024 * 1024, // 1MB
            max_array_size: 10_000,
            max_map_size: 10_000,
            hot_reload: false,
            extension: "rhai".to_string(),
        }
    }
}

impl RhaiEngine {
    /// Create a new engine builder with default configuration.
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> RhaiEngineBuilder {
        RhaiEngineBuilder::new()
    }

    /// Create engine from configuration.
    pub fn from_config(config: EngineConfig) -> Result<Self> {
        let mut engine = Engine::new();

        // Apply limits
        if let Some(max_ops) = config.max_operations {
            engine.set_max_operations(max_ops);
        }
        engine.set_max_call_levels(config.max_call_depth);
        engine.set_max_string_size(config.max_string_size);
        engine.set_max_array_size(config.max_array_size);
        engine.set_max_map_size(config.max_map_size);

        // Register Armature API
        register_armature_api(&mut engine);

        // Create script loader
        let loader = ScriptLoader::new(&config.scripts_dir);

        Ok(Self {
            engine,
            cache: Arc::new(RwLock::new(ScriptCache::new())),
            loader,
            config,
        })
    }

    /// Get a reference to the underlying Rhai engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Get a mutable reference to the underlying Rhai engine.
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Get the configuration.
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Compile a script from a file.
    pub fn compile_file(&self, path: impl AsRef<Path>) -> Result<Arc<CompiledScript>> {
        let path = path.as_ref();
        let full_path = self.loader.resolve_path(path);

        if self.config.hot_reload {
            // In hot-reload mode, evict the cached script for *this*
            // request's path if its source file has changed since it was
            // compiled (`CompiledScript::is_stale` via
            // `ScriptCache::evict_if_stale`), so a stale AST is never
            // served and the cache lookup below naturally falls through to
            // a fresh recompilation. An unchanged file is left in the cache
            // and served as normal.
            //
            // This only stats the one entry named by `full_path` — not
            // every cached script (that's what `evict_stale()` does) — so
            // it stays cheap even under load with a large cache, instead of
            // doing a full filesystem stat sweep on every request.
            self.cache.write().evict_if_stale(&full_path);
        }

        // Check cache first.
        if let Some(script) = self.cache.read().get(&full_path) {
            return Ok(script);
        }

        // Load and compile
        let source = self.loader.load(&full_path)?;
        let ast = self
            .engine
            .compile(&source)
            .map_err(|e| RhaiError::compilation(path, e.to_string()))?;

        let script = Arc::new(CompiledScript::new(full_path.clone(), ast));

        // Cache the compiled script
        self.cache.write().insert(full_path, script.clone());

        Ok(script)
    }

    /// Compile a script from source code.
    pub fn compile(&self, source: &str) -> Result<AST> {
        self.engine
            .compile(source)
            .map_err(|e| RhaiError::Parse(e.to_string()))
    }

    /// Execute a compiled script with the given scope.
    pub fn run(&self, script: &CompiledScript, scope: &mut Scope) -> Result<Dynamic> {
        self.engine
            .run_ast_with_scope(scope, script.ast())
            .map_err(|e| RhaiError::runtime(&script.path, e.to_string()))?;

        // Get the response from scope
        if let Some(response) = scope.get_value::<crate::bindings::ResponseBinding>("response") {
            Ok(Dynamic::from(response))
        } else {
            Ok(Dynamic::UNIT)
        }
    }

    /// Execute a compiled script and return the result.
    pub fn eval(&self, script: &CompiledScript, scope: &mut Scope) -> Result<Dynamic> {
        self.engine
            .eval_ast_with_scope(scope, script.ast())
            .map_err(|e| RhaiError::runtime(&script.path, e.to_string()))
    }

    /// Execute a script file with the given scope.
    pub fn run_file(&self, path: impl AsRef<Path>, scope: &mut Scope) -> Result<Dynamic> {
        let script = self.compile_file(path)?;
        self.run(&script, scope)
    }

    /// Clear the script cache.
    pub fn clear_cache(&self) {
        self.cache.write().clear();
    }

    /// Invalidate a specific script in the cache.
    pub fn invalidate(&self, path: impl AsRef<Path>) {
        let full_path = self.loader.resolve_path(path.as_ref());
        self.cache.write().remove(&full_path);
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.read().stats()
    }
}

impl Default for RhaiEngine {
    fn default() -> Self {
        RhaiEngineBuilder::new().build().expect("default engine")
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of cached scripts.
    pub cached_scripts: usize,
    /// Total cache hits.
    pub hits: u64,
    /// Total cache misses.
    pub misses: u64,
}

/// Builder for RhaiEngine.
pub struct RhaiEngineBuilder {
    config: EngineConfig,
}

impl Default for RhaiEngineBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RhaiEngineBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self {
            config: EngineConfig::default(),
        }
    }

    /// Set the scripts directory.
    pub fn scripts_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.scripts_dir = path.into();
        self
    }

    /// Set maximum operations per script.
    pub fn max_operations(mut self, max: u64) -> Self {
        self.config.max_operations = Some(max);
        self
    }

    /// Disable operation limit.
    pub fn unlimited_operations(mut self) -> Self {
        self.config.max_operations = None;
        self
    }

    /// Set maximum call stack depth.
    pub fn max_call_depth(mut self, depth: usize) -> Self {
        self.config.max_call_depth = depth;
        self
    }

    /// Set maximum string size.
    pub fn max_string_size(mut self, size: usize) -> Self {
        self.config.max_string_size = size;
        self
    }

    /// Set maximum array size.
    pub fn max_array_size(mut self, size: usize) -> Self {
        self.config.max_array_size = size;
        self
    }

    /// Set maximum map size.
    pub fn max_map_size(mut self, size: usize) -> Self {
        self.config.max_map_size = size;
        self
    }

    /// Enable hot reload for development.
    pub fn hot_reload(mut self, enabled: bool) -> Self {
        self.config.hot_reload = enabled;
        self
    }

    /// Set script file extension.
    pub fn extension(mut self, ext: impl Into<String>) -> Self {
        self.config.extension = ext.into();
        self
    }

    /// Build the engine.
    pub fn build(self) -> Result<RhaiEngine> {
        RhaiEngine::from_config(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    #[test]
    fn test_hot_reload_recompiles_only_when_stale() {
        let temp = tempfile::TempDir::new().unwrap();
        let script_path = temp.path().join("route.rhai");
        fs::write(&script_path, "40 + 2").unwrap();

        let engine = RhaiEngine::new()
            .scripts_dir(temp.path())
            .hot_reload(true)
            .build()
            .unwrap();

        let first = engine.compile_file("route.rhai").unwrap();

        // Re-compiling an unchanged file must be served from cache, not
        // recompiled unconditionally just because hot-reload is on.
        let second = engine.compile_file("route.rhai").unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "unchanged script should be served from cache under hot-reload"
        );

        // Change the file's contents and bump its mtime forward so it is
        // unambiguously newer than the first compilation (avoids relying on
        // filesystem mtime resolution / real-time sleeps).
        fs::write(&script_path, "1 + 1").unwrap();
        let future = first.compiled_at() + Duration::from_secs(2);
        let file = fs::File::options().write(true).open(&script_path).unwrap();
        file.set_modified(future).unwrap();

        let third = engine.compile_file("route.rhai").unwrap();
        assert!(
            !Arc::ptr_eq(&first, &third),
            "changed script must be recompiled under hot-reload"
        );

        // Confirm the *new* source actually got compiled.
        let mut scope = Scope::new();
        let value = engine.eval(&third, &mut scope).unwrap();
        assert_eq!(value.as_int().unwrap(), 2);
    }

    #[test]
    fn test_without_hot_reload_cache_is_never_invalidated_by_mtime() {
        let temp = tempfile::TempDir::new().unwrap();
        let script_path = temp.path().join("route.rhai");
        fs::write(&script_path, "40 + 2").unwrap();

        let engine = RhaiEngine::new().scripts_dir(temp.path()).build().unwrap();

        let first = engine.compile_file("route.rhai").unwrap();

        fs::write(&script_path, "1 + 1").unwrap();
        let future = first.compiled_at() + Duration::from_secs(2);
        let file = fs::File::options().write(true).open(&script_path).unwrap();
        file.set_modified(future).unwrap();

        let second = engine.compile_file("route.rhai").unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "without hot-reload, a changed file must still be served from cache"
        );
    }
}
