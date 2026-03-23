//! Script loading and caching.

use crate::error::{Result, RhaiError};
use dashmap::DashMap;
use rhai::AST;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// A compiled Rhai script.
#[derive(Debug)]
pub struct CompiledScript {
    /// Path to the script file.
    pub path: PathBuf,
    /// Compiled AST.
    ast: AST,
    /// Compilation timestamp.
    compiled_at: SystemTime,
}

impl CompiledScript {
    /// Create a new compiled script.
    pub fn new(path: PathBuf, ast: AST) -> Self {
        Self {
            path,
            ast,
            compiled_at: SystemTime::now(),
        }
    }

    /// Get the compiled AST.
    pub fn ast(&self) -> &AST {
        &self.ast
    }

    /// Get the compilation timestamp.
    pub fn compiled_at(&self) -> SystemTime {
        self.compiled_at
    }

    /// Check if the script is stale (source file modified).
    pub fn is_stale(&self) -> bool {
        if let Ok(metadata) = fs::metadata(&self.path) {
            if let Ok(modified) = metadata.modified() {
                return modified > self.compiled_at;
            }
        }
        false
    }
}

/// Cache for compiled scripts.
pub struct ScriptCache {
    scripts: HashMap<PathBuf, Arc<CompiledScript>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl Default for ScriptCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            scripts: HashMap::new(),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Get a cached script.
    pub fn get(&self, path: &Path) -> Option<Arc<CompiledScript>> {
        if let Some(script) = self.scripts.get(path) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(script.clone())
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Insert a script into the cache.
    pub fn insert(&mut self, path: PathBuf, script: Arc<CompiledScript>) {
        self.scripts.insert(path, script);
    }

    /// Remove a script from the cache.
    pub fn remove(&mut self, path: &Path) -> Option<Arc<CompiledScript>> {
        self.scripts.remove(path)
    }

    /// Clear all cached scripts.
    pub fn clear(&mut self) {
        self.scripts.clear();
    }

    /// Get cache statistics.
    pub fn stats(&self) -> crate::engine::CacheStats {
        crate::engine::CacheStats {
            cached_scripts: self.scripts.len(),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }

    /// Remove stale scripts from the cache.
    pub fn evict_stale(&mut self) -> Vec<PathBuf> {
        let stale: Vec<PathBuf> = self
            .scripts
            .iter()
            .filter(|(_, script)| script.is_stale())
            .map(|(path, _)| path.clone())
            .collect();

        for path in &stale {
            self.scripts.remove(path);
        }

        stale
    }
}

/// Concurrent script cache for multi-threaded access.
pub struct ConcurrentScriptCache {
    scripts: DashMap<PathBuf, Arc<CompiledScript>>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl Default for ConcurrentScriptCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ConcurrentScriptCache {
    /// Create a new concurrent cache.
    pub fn new() -> Self {
        Self {
            scripts: DashMap::new(),
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        }
    }

    /// Get a cached script.
    pub fn get(&self, path: &Path) -> Option<Arc<CompiledScript>> {
        if let Some(script) = self.scripts.get(path) {
            self.hits.fetch_add(1, Ordering::Relaxed);
            Some(script.clone())
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            None
        }
    }

    /// Insert a script into the cache.
    pub fn insert(&self, path: PathBuf, script: Arc<CompiledScript>) {
        self.scripts.insert(path, script);
    }

    /// Remove a script from the cache.
    pub fn remove(&self, path: &Path) -> Option<Arc<CompiledScript>> {
        self.scripts.remove(path).map(|(_, v)| v)
    }

    /// Clear all cached scripts.
    pub fn clear(&self) {
        self.scripts.clear();
    }
}

/// Script loader for file operations.
pub struct ScriptLoader {
    /// Base directory for scripts.
    base_dir: PathBuf,
}

impl ScriptLoader {
    /// Create a new script loader.
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Resolve a script path relative to the base directory.
    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base_dir.join(path)
        }
    }

    /// Load a script file.
    pub fn load(&self, path: &Path) -> Result<String> {
        let full_path = self.resolve_path(path);

        if !full_path.exists() {
            return Err(RhaiError::ScriptNotFound { path: full_path });
        }

        fs::read_to_string(&full_path).map_err(RhaiError::from)
    }

    /// Check if a script exists.
    pub fn exists(&self, path: &Path) -> bool {
        self.resolve_path(path).exists()
    }

    /// List all script files in a directory.
    pub fn list_scripts(&self, dir: &Path, extension: &str) -> Result<Vec<PathBuf>> {
        let full_path = self.resolve_path(dir);
        let mut scripts = Vec::new();

        if !full_path.exists() {
            return Ok(scripts);
        }

        self.collect_scripts(&full_path, extension, &mut scripts)?;
        Ok(scripts)
    }

    fn collect_scripts(
        &self,
        dir: &Path,
        extension: &str,
        scripts: &mut Vec<PathBuf>,
    ) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                self.collect_scripts(&path, extension, scripts)?;
            } else if path.extension().map(|e| e == extension).unwrap_or(false) {
                scripts.push(path);
            }
        }
        Ok(())
    }

    /// Get the base directory.
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_script_loader() {
        let temp = TempDir::new().unwrap();
        let mut file = fs::File::create(temp.path().join("test.rhai")).unwrap();
        writeln!(file, "let x = 42;").unwrap();

        let loader = ScriptLoader::new(temp.path());
        let content = loader.load(Path::new("test.rhai")).unwrap();
        assert!(content.contains("42"));
    }

    #[test]
    fn test_script_cache() {
        use rhai::Engine;

        let engine = Engine::new();
        let ast = engine.compile("let x = 1;").unwrap();
        let script = Arc::new(CompiledScript::new(PathBuf::from("test.rhai"), ast));

        let mut cache = ScriptCache::new();
        cache.insert(PathBuf::from("test.rhai"), script.clone());

        assert!(cache.get(Path::new("test.rhai")).is_some());
        assert!(cache.get(Path::new("other.rhai")).is_none());

        let stats = cache.stats();
        assert_eq!(stats.cached_scripts, 1);
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
    }
}
