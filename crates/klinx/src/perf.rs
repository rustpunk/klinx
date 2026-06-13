//! Opt-in performance tracing, compiled out unless the `perf-trace` feature is
//! enabled. See `docs/perf.md` for how the timings are used.

/// Time `$body` and return its value.
///
/// With the `perf-trace` feature on, prints `[perf] <label> in <elapsed>` to
/// stderr; with it off, this expands to just `$body` — the label and timing are
/// removed by `#[cfg]` before type-checking, so there is no runtime cost (not
/// even argument evaluation). Because the label is only expanded inside the
/// gated `eprintln!`, it may reference inputs that are still in scope after
/// `$body` (e.g. an input slice's length).
///
/// The label uses `format_args!` syntax: `perf_trace!(work(), "tag: {}", n)`.
macro_rules! perf_trace {
    ($body:expr, $($label:tt)*) => {{
        #[cfg(feature = "perf-trace")]
        let __perf_start = std::time::Instant::now();
        let __perf_result = $body;
        #[cfg(feature = "perf-trace")]
        eprintln!(
            "[perf] {} in {:?}",
            format_args!($($label)*),
            __perf_start.elapsed()
        );
        __perf_result
    }};
}

pub(crate) use perf_trace;
