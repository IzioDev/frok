use anyhow::{Context, Result};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub struct LoggingGuard {
    _file_guard: Option<WorkerGuard>,
}

#[derive(Clone, Copy)]
pub struct LoggingConfig<'a> {
    pub file_prefix: Option<&'a str>,
    pub with_stdout: bool,
}

pub fn init_logging(config: LoggingConfig<'_>) -> Result<LoggingGuard> {
    let filter = EnvFilter::try_from_env("FROK_LOG")
        .or_else(|_| EnvFilter::try_from_env("RUST_LOG"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let mut file_guard = None;

    let file_layer = if let Some(prefix) = config.file_prefix {
        let log_dir = std::env::current_dir()
            .context("get current dir for log files")?
            .join("logs");
        std::fs::create_dir_all(&log_dir).context("create logs directory")?;

        let file_appender = tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(prefix)
            .filename_suffix(".txt")
            .build(&log_dir)
            .context("build file appender")?;
        let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
        file_guard = Some(guard);

        Some(
            fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false)
                .with_target(true)
                .with_thread_ids(true),
        )
    } else {
        None
    };

    let stdout_layer = if config.with_stdout {
        Some(
            fmt::layer()
                .with_writer(std::io::stdout)
                .with_ansi(true)
                .with_target(false),
        )
    } else {
        None
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stdout_layer)
        .init();

    Ok(LoggingGuard {
        _file_guard: file_guard,
    })
}
