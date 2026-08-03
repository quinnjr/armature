// Mock utilities for testing

use armature_core::Provider;
use std::sync::{Arc, Mutex};

/// Mock service for testing
#[derive(Clone)]
pub struct MockService<T> {
    calls: Arc<Mutex<Vec<String>>>,
    return_value: Arc<Mutex<Option<T>>>,
}

impl<T> MockService<T> {
    /// Create a new mock service
    pub fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            return_value: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the return value
    pub fn with_return(self, value: T) -> Self {
        *self.return_value.lock().unwrap() = Some(value);
        self
    }

    /// Record a method call
    pub fn record_call(&self, method: &str) {
        self.calls.lock().unwrap().push(method.to_string());
    }

    /// Get the number of calls
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Get all recorded calls
    pub fn get_calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    /// Check if a method was called
    pub fn was_called(&self, method: &str) -> bool {
        self.calls.lock().unwrap().contains(&method.to_string())
    }

    /// Clear all recorded calls
    pub fn clear_calls(&self) {
        self.calls.lock().unwrap().clear();
    }

    /// Get the mock return value
    pub fn get_return(&self) -> Option<T>
    where
        T: Clone,
    {
        self.return_value.lock().unwrap().clone()
    }
}

impl<T> Default for MockService<T> {
    fn default() -> Self {
        Self::new()
    }
}

type CallLog = Arc<Mutex<Vec<(String, Vec<String>)>>>;

/// Mock controller for testing
#[derive(Clone)]
pub struct MockController {
    _name: String,
    calls: CallLog,
}

impl MockController {
    /// Create a new mock controller
    pub fn new(name: &str) -> Self {
        Self {
            _name: name.to_string(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Record a method call with arguments
    pub fn record_call(&self, method: &str, args: Vec<String>) {
        self.calls.lock().unwrap().push((method.to_string(), args));
    }

    /// Get the number of calls to a specific method
    pub fn method_call_count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .count()
    }

    /// Get all calls
    pub fn get_all_calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }

    /// Clear all calls
    pub fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }
}

/// Mock provider trait implementation
pub trait MockProvider: Provider + Clone {
    /// Reset the mock to its initial state
    fn reset(&mut self);

    /// Get the number of calls
    fn call_count(&self) -> usize;
}

/// Call recorder that carries a wrapped value alongside it.
///
/// # This does not intercept anything
///
/// `Spy` is a **manual** recorder, not a proxy. Wrapping a value does not make
/// its methods observable: [`Spy::inner`] hands back the value untouched, and
/// calls made through that reference go straight to it. Recording happens only
/// where you call [`Spy::record`] yourself:
///
/// ```
/// # use armature_testing::Spy;
/// #[derive(Clone)]
/// struct Repo;
/// impl Repo {
///     fn find(&self, id: u32) -> String { format!("user-{id}") }
/// }
///
/// let spy = Spy::new(Repo);
///
/// // The call is recorded because the test records it, not because `Spy` saw it.
/// spy.record("find");
/// let user = spy.inner().find(7);
///
/// assert_eq!(user, "user-7");
/// assert_eq!(spy.call_count(), 1);
/// assert!(spy.was_called("find"));
/// ```
///
/// For automatic interception, implement the trait under test on a hand-written
/// fake that records in each method - `Spy` is then useful as that fake's
/// storage.
///
/// Cloning a `Spy` shares the recording buffer, so a clone handed to production
/// code records into the same log the test asserts on.
#[derive(Clone)]
pub struct Spy<T: Clone> {
    inner: T,
    calls: Arc<Mutex<Vec<String>>>,
}

impl<T: Clone> Spy<T> {
    /// Create a new spy wrapping a provider
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Record a method call.
    ///
    /// Call this yourself at each site you want observed; `Spy` cannot see
    /// calls made through [`Spy::inner`].
    pub fn record(&self, method: &str) {
        self.calls.lock().unwrap().push(method.to_string());
    }

    /// Get the number of recorded calls
    pub fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// Check if a method was recorded as called
    pub fn was_called(&self, method: &str) -> bool {
        self.calls.lock().unwrap().contains(&method.to_string())
    }

    /// Every recorded call, in order.
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    /// Drop every recorded call.
    pub fn clear(&self) {
        self.calls.lock().unwrap().clear();
    }

    /// Get the wrapped provider.
    ///
    /// Returned untouched - calls made through this reference are NOT
    /// recorded. See the type-level docs.
    pub fn inner(&self) -> &T {
        &self.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_service() {
        let mock = MockService::<String>::new();
        mock.record_call("test_method");
        assert_eq!(mock.call_count(), 1);
        assert!(mock.was_called("test_method"));
    }

    #[test]
    fn test_mock_controller() {
        let mock = MockController::new("TestController");
        mock.record_call("get", vec!["id".to_string()]);
        assert_eq!(mock.method_call_count("get"), 1);
    }

    #[test]
    fn test_spy() {
        let spy = Spy::new("test_value");
        spy.record("method1");
        spy.record("method2");
        assert_eq!(spy.call_count(), 2);
        assert!(spy.was_called("method1"));
    }
}
