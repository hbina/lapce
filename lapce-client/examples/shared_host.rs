//! shared_host <lapce-proxy executable> <workspace> <socket path>

fn main() -> anyhow::Result<()> {
    use lapce_client::{
        ConnectionOptions, LocalBackend, shared::SharedBackend,
        workspace::LapceWorkspace,
    };
    use std::{
        io::{self, BufRead},
        os::unix::{fs::PermissionsExt, net::UnixListener},
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };
    let mut args = std::env::args_os().skip(1);
    let executable = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("expected proxy executable"))?,
    );
    let workspace = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("expected workspace"))?,
    )
    .canonicalize()?;
    let socket = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("expected socket path"))?,
    );
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let host = SharedBackend::connect(ConnectionOptions {
        workspace: LapceWorkspace {
            path: Some(workspace),
            ..Default::default()
        },
        local_backend: LocalBackend::Process { executable },
        ..Default::default()
    })?;
    let stop = Arc::new(AtomicBool::new(false));
    let input_stop = stop.clone();
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            if matches!(line.as_deref(), Ok("quit")) {
                input_stop.store(true, Ordering::Release);
                break;
            }
        }
    });
    println!(
        "Shared backend listening at {}. Enter quit to stop.",
        socket.display()
    );
    let mut workers = Vec::new();
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => {
                let frontend = host.attach()?;
                workers.push(std::thread::spawn(move || {
                    lapce_client::socket::serve_frontend(frontend, stream)
                }));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error.into()),
        }
    }
    host.shutdown_and_wait()?;
    for worker in workers {
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("frontend worker panicked"))??;
    }
    drop(listener);
    std::fs::remove_file(socket)?;
    Ok(())
}
