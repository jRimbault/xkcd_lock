use std::path::PathBuf;

use tracing_subscriber::{
    filter::{LevelFilter, Targets},
    layer::SubscriberExt,
    util::SubscriberInitExt,
    Layer,
};

pub fn init(verbosity: &clap_verbosity_flag::Verbosity) -> anyhow::Result<()> {
    let default_filter = verbosity
        .log_level_filter()
        .to_string()
        .to_ascii_lowercase();
    let trace_log_path = trace_log_path();
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&trace_log_path)
        .map_err(anyhow::Error::from)
        .and_then(|file| {
            let trace_filter = trace_log_filter();
            tracing_subscriber::registry()
                .with(tracing_subscriber::fmt::layer().with_filter(console_filter(default_filter)))
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(move || {
                            file.try_clone()
                                .expect("trace log file handle should stay cloneable")
                        })
                        .with_filter(trace_filter),
                )
                .try_init()
                .map_err(anyhow::Error::from)
        })
        .map_err(|error| {
            anyhow::anyhow!(
                "failed to initialize tracing with trace log at {}: {error}",
                trace_log_path.display()
            )
        })?;
    tracing::trace!(path = %trace_log_path.display(), "Configured trace log file");
    Ok(())
}

fn console_filter(default_filter: String) -> tracing_subscriber::EnvFilter {
    std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| tracing_subscriber::EnvFilter::new(default_filter))
}

fn trace_log_path() -> PathBuf {
    // Allow tests to redirect the unconditional trace log away from shared /tmp.
    std::env::var_os("XKCD_LOCK_TRACE_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp/xkcd_lock.trace.log"))
}

fn trace_log_filter() -> Targets {
    Targets::new()
        .with_default(LevelFilter::TRACE)
        .with_target("ureq", LevelFilter::INFO)
        .with_target("ureq_proto", LevelFilter::INFO)
        .with_target("rustls", LevelFilter::INFO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_log_filter_limits_noisy_dependencies() {
        assert_eq!(
            trace_log_filter(),
            Targets::new()
                .with_default(LevelFilter::TRACE)
                .with_target("ureq", LevelFilter::INFO)
                .with_target("ureq_proto", LevelFilter::INFO)
                .with_target("rustls", LevelFilter::INFO)
        );
    }
}
