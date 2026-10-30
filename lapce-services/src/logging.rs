//! One queued logging worker per process, shared by frontend and embedded backend.
use std::io::{self, Write};

use lapce_core::directory::Directory;
use tracing::level_filters::LevelFilter;
use tracing_appender::{non_blocking::WorkerGuard, rolling::RollingFileAppender};
use tracing_subscriber::{filter::Targets, prelude::*, reload::Handle};

struct Outputs {
    file: Option<RollingFileAppender>,
}
impl Write for Outputs {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        // A failed file sink must not hide logs from the launching terminal.
        let console = io::stderr().write_all(bytes);
        let file = self.file.as_mut().map(|file| file.write_all(bytes));
        if console.is_err() && !matches!(file, Some(Ok(()))) {
            console?;
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        let _ = io::stderr().flush();
        if let Some(file) = &mut self.file {
            file.flush()?;
        }
        Ok(())
    }
}

/// Keep the guard alive until application shutdown so queued records are flushed.
pub fn init(program: &str) -> (Handle<Targets>, WorkerGuard) {
    let file = Directory::logs_directory().and_then(|dir| {
        tracing_appender::rolling::Builder::new()
            .max_log_files(10)
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(program)
            .filename_suffix("log")
            .build(dir)
            .ok()
    });
    let (writer, guard) =
        tracing_appender::non_blocking::NonBlockingBuilder::default()
            .thread_name("Lapce logs")
            .finish(Outputs { file });
    let targets = std::env::var("LAPCE_LOG")
        .ok()
        .and_then(|value| value.parse::<Targets>().ok())
        .unwrap_or_else(|| {
            Targets::new()
                .with_default(LevelFilter::WARN)
                .with_target("lapce_app", LevelFilter::INFO)
                .with_target("lapce_iced", LevelFilter::INFO)
                .with_target("lapce_client", LevelFilter::INFO)
                .with_target("lapce_proxy", LevelFilter::INFO)
                .with_target("lapce_engine", LevelFilter::INFO)
                .with_target("lapce_services", LevelFilter::INFO)
        });
    let (filter, handle) = tracing_subscriber::reload::Subscriber::new(targets);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::subscriber()
                .with_writer(writer)
                .with_ansi(false)
                .with_target(true)
                .with_thread_names(true)
                .with_thread_ids(true)
                .with_filter(filter),
        )
        .init();
    tracing::info!(
        program,
        pid = std::process::id(),
        "Logging initialized; foreground process"
    );
    (handle, guard)
}
