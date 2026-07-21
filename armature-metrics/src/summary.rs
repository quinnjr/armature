//! Summary metrics
//!
//! Summaries sample observations and expose configurable φ-quantiles computed
//! over a sliding time window, alongside a cumulative sample count and sum.
//!
//! The upstream `prometheus` crate does not ship a built-in `Summary` metric
//! type (it only exposes the wire-format protobuf message), so this module
//! implements a real one as a [`prometheus::core::Collector`]: it can be
//! registered into any registry and is emitted by the text encoder as a
//! `SUMMARY` metric family.

use prometheus::core::{Collector, Desc};
use prometheus::proto;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Default quantiles reported when none are configured.
pub const DEFAULT_QUANTILES: &[f64] = &[0.5, 0.9, 0.99];

/// Default sliding window over which quantiles are computed (10 minutes).
pub const DEFAULT_MAX_AGE: Duration = Duration::from_secs(600);

/// Default upper bound on the number of buffered observations kept for quantile
/// estimation. Bounds memory regardless of observation rate.
pub const DEFAULT_MAX_SIZE: usize = 5000;

/// Configuration for a [`Summary`].
#[derive(Clone, Debug)]
pub struct SummaryOpts {
    /// Fully-qualified metric name.
    pub name: String,
    /// Metric help text.
    pub help: String,
    /// Quantiles to report (each in `0.0..=1.0`).
    pub quantiles: Vec<f64>,
    /// Sliding window over which quantiles are computed.
    pub max_age: Duration,
    /// Maximum number of buffered observations.
    pub max_size: usize,
}

impl SummaryOpts {
    /// Create options with the default quantiles, window and buffer size.
    pub fn new(name: impl Into<String>, help: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            help: help.into(),
            quantiles: DEFAULT_QUANTILES.to_vec(),
            max_age: DEFAULT_MAX_AGE,
            max_size: DEFAULT_MAX_SIZE,
        }
    }

    /// Override the reported quantiles.
    pub fn quantiles(mut self, quantiles: Vec<f64>) -> Self {
        self.quantiles = quantiles;
        self
    }

    /// Override the sliding window over which quantiles are computed.
    pub fn max_age(mut self, max_age: Duration) -> Self {
        self.max_age = max_age;
        self
    }

    /// Override the maximum number of buffered observations.
    pub fn max_size(mut self, max_size: usize) -> Self {
        self.max_size = max_size;
        self
    }
}

/// Mutable state shared behind a lock.
#[derive(Debug)]
struct SummaryState {
    /// Buffered `(observed_at, value)` pairs used for quantile estimation.
    observations: VecDeque<(Instant, f64)>,
    /// Cumulative count of every observation ever recorded.
    count: u64,
    /// Cumulative sum of every observation ever recorded.
    sum: f64,
}

/// A summary metric.
///
/// Cheaply cloneable; all clones share the same underlying state, so a clone can
/// be registered while another is used to [`observe`](Summary::observe).
///
/// # Examples
///
/// ```
/// use armature_metrics::Summary;
///
/// let summary = Summary::new("request_latency_seconds", "Request latency").unwrap();
/// summary.observe(0.2);
/// summary.observe(0.4);
/// summary.observe(0.6);
///
/// assert_eq!(summary.get_sample_count(), 3);
/// assert!((summary.get_sample_sum() - 1.2).abs() < 1e-9);
/// // The 0.5 quantile of {0.2, 0.4, 0.6} is 0.4.
/// assert_eq!(summary.quantile(0.5), Some(0.4));
/// ```
#[derive(Clone)]
pub struct Summary {
    desc: Arc<Desc>,
    quantiles: Arc<Vec<f64>>,
    max_age: Duration,
    max_size: usize,
    state: Arc<Mutex<SummaryState>>,
}

impl Summary {
    /// Create a new summary with the default quantiles and sliding window.
    pub fn new(
        name: impl Into<String>,
        help: impl Into<String>,
    ) -> Result<Summary, prometheus::Error> {
        Self::with_opts(SummaryOpts::new(name, help))
    }

    /// Create a new summary from explicit [`SummaryOpts`].
    pub fn with_opts(opts: SummaryOpts) -> Result<Summary, prometheus::Error> {
        let desc = Desc::new(opts.name, opts.help, vec![], HashMap::new())?;
        Ok(Summary {
            desc: Arc::new(desc),
            quantiles: Arc::new(opts.quantiles),
            max_age: opts.max_age,
            max_size: opts.max_size.max(1),
            state: Arc::new(Mutex::new(SummaryState {
                observations: VecDeque::new(),
                count: 0,
                sum: 0.0,
            })),
        })
    }

    /// Record an observation.
    pub fn observe(&self, value: f64) {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.count += 1;
        state.sum += value;
        state.observations.push_back((now, value));
        self.trim(&mut state, now);
    }

    /// Cumulative number of observations recorded over the summary's lifetime.
    pub fn get_sample_count(&self) -> u64 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).count
    }

    /// Cumulative sum of every observation recorded over the summary's lifetime.
    pub fn get_sample_sum(&self) -> f64 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).sum
    }

    /// Estimate the `q` quantile over the current sliding window.
    ///
    /// Returns `None` when no observations are within the window.
    pub fn quantile(&self, q: f64) -> Option<f64> {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        self.trim(&mut state, now);
        let mut values: Vec<f64> = state.observations.iter().map(|(_, v)| *v).collect();
        drop(state);
        if values.is_empty() {
            return None;
        }
        // Single quantile: an O(n) partial-sort (`select_nth_unstable_by`) puts
        // the nearest-rank element at its final index without fully sorting the
        // buffer, which the old `sort_by` did on every call. The index is the
        // same nearest-rank position `quantile_of` computes on a sorted slice.
        let n = values.len();
        let q = q.clamp(0.0, 1.0);
        let rank = (q * n as f64).ceil() as usize;
        let idx = rank.saturating_sub(1).min(n - 1);
        let (_, nth, _) = values.select_nth_unstable_by(idx, |a, b| {
            a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
        });
        Some(*nth)
    }

    /// Drop observations older than `max_age` and enforce the size bound.
    fn trim(&self, state: &mut SummaryState, now: Instant) {
        while let Some(&(t, _)) = state.observations.front() {
            if now.duration_since(t) > self.max_age {
                state.observations.pop_front();
            } else {
                break;
            }
        }
        while state.observations.len() > self.max_size {
            state.observations.pop_front();
        }
    }
}

/// Nearest-rank φ-quantile of a pre-sorted, non-empty slice.
fn quantile_of(sorted: &[f64], q: f64) -> f64 {
    debug_assert!(!sorted.is_empty());
    let n = sorted.len();
    let q = q.clamp(0.0, 1.0);
    let rank = (q * n as f64).ceil() as usize;
    let idx = rank.saturating_sub(1).min(n - 1);
    sorted[idx]
}

impl Collector for Summary {
    fn desc(&self) -> Vec<&Desc> {
        vec![self.desc.as_ref()]
    }

    fn collect(&self) -> Vec<proto::MetricFamily> {
        let now = Instant::now();
        let (count, sum, mut values) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            self.trim(&mut state, now);
            let values: Vec<f64> = state.observations.iter().map(|(_, v)| *v).collect();
            (state.count, state.sum, values)
        };
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let mut quantiles = Vec::with_capacity(self.quantiles.len());
        for &q in self.quantiles.iter() {
            let value = if values.is_empty() {
                f64::NAN
            } else {
                quantile_of(&values, q)
            };
            let mut quantile = proto::Quantile::default();
            quantile.set_quantile(q);
            quantile.set_value(value);
            quantiles.push(quantile);
        }

        let mut summary = proto::Summary::default();
        summary.set_sample_count(count);
        summary.set_sample_sum(sum);
        summary.set_quantile(quantiles);

        let mut metric = proto::Metric::from_label(vec![]);
        metric.set_summary(summary);

        let mut mf = proto::MetricFamily::default();
        mf.set_name(self.desc.fq_name.clone());
        mf.set_help(self.desc.help.clone());
        mf.set_field_type(proto::MetricType::SUMMARY);
        mf.set_metric(vec![metric]);

        vec![mf]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::Registry;

    #[test]
    fn test_observe_tracks_count_and_sum() {
        let summary = Summary::new("test_summary_count_sum", "help").unwrap();
        summary.observe(1.0);
        summary.observe(2.0);
        summary.observe(3.0);
        assert_eq!(summary.get_sample_count(), 3);
        assert!((summary.get_sample_sum() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn test_quantiles_are_computed() {
        let summary = Summary::new("test_summary_quantiles", "help").unwrap();
        assert_eq!(summary.quantile(0.5), None);
        for i in 1..=100 {
            summary.observe(i as f64);
        }
        // Nearest-rank median of 1..=100 is 50.
        assert_eq!(summary.quantile(0.5), Some(50.0));
        assert_eq!(summary.quantile(0.99), Some(99.0));
        assert_eq!(summary.quantile(1.0), Some(100.0));
    }

    #[test]
    fn test_summary_registers_and_exports() {
        let registry = Registry::new();
        let summary = Summary::with_opts(SummaryOpts::new(
            "test_summary_export",
            "exported summary help",
        ))
        .unwrap();
        registry.register(Box::new(summary.clone())).unwrap();
        summary.observe(0.5);
        summary.observe(1.5);

        let text = crate::export_metrics_from_registry(&registry);
        assert!(text.contains("test_summary_export"), "got:\n{text}");
        assert!(text.contains("test_summary_export_count"), "got:\n{text}");
        assert!(text.contains("test_summary_export_sum"), "got:\n{text}");
        assert!(text.contains("quantile="), "got:\n{text}");
    }

    #[test]
    fn test_max_size_bounds_buffer_but_not_totals() {
        let summary =
            Summary::with_opts(SummaryOpts::new("test_summary_bounded", "help").max_size(10))
                .unwrap();
        for i in 0..1000 {
            summary.observe(i as f64);
        }
        // Cumulative totals reflect every observation...
        assert_eq!(summary.get_sample_count(), 1000);
        // ...but only the most recent values remain for quantile estimation.
        assert_eq!(summary.quantile(1.0), Some(999.0));
        assert_eq!(summary.quantile(0.0), Some(990.0));
    }
}
