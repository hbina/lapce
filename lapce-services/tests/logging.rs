use std::{path::Path, process::Command};

#[test]
fn logging_child() {
    if std::env::var_os("LAPCE_LOG_TEST_CHILD").is_none() {
        return;
    }
    let (_reload, _guard) = lapce_services::logging::init("startup-test");
    println!("stdout-protocol-marker");
    tracing::info!(target: "lapce_services", "frontend-info-marker");
    std::thread::Builder::new()
        .name("Lapce backend".into())
        .spawn(|| {
            tracing::info!(target: "lapce_services", "backend-info-marker");
            tracing::debug!(target: "lapce_services", "backend-debug-marker");
        })
        .unwrap()
        .join()
        .unwrap();
}

fn log_files(path: &Path) -> String {
    let mut logs = String::new();
    if let Ok(entries) = path.read_dir() {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                logs.push_str(&log_files(&entry.path()));
            } else if entry
                .file_name()
                .to_string_lossy()
                .starts_with("startup-test")
            {
                logs.push_str(&std::fs::read_to_string(entry.path()).unwrap());
            }
        }
    }
    logs
}

#[test]
fn terminal_logs_flush_and_respect_filters_even_when_file_logging_fails() {
    for file_logging in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let data = directory.path().join("data");
        if !file_logging {
            std::fs::write(&data, "not a directory").unwrap();
        }
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "logging_child", "--nocapture"])
            .env("LAPCE_LOG_TEST_CHILD", "1")
            .env("XDG_DATA_HOME", &data)
            .env_remove("LAPCE_LOG");
        if file_logging {
            command.env("LAPCE_LOG", "lapce_services=debug,off");
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("stdout-protocol-marker"));
        assert!(!stdout.contains("frontend-info-marker"));
        for marker in [
            "frontend-info-marker",
            "backend-info-marker",
            "Lapce backend",
        ] {
            assert!(stderr.contains(marker), "missing {marker}: {stderr}");
        }
        assert_eq!(stderr.contains("backend-debug-marker"), file_logging);
        if file_logging {
            let logs = log_files(directory.path());
            assert!(
                logs.contains("backend-debug-marker"),
                "log worker did not flush the file"
            );
        }
    }
}
