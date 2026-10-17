//! The stdio client adapter shared by local child processes and SSH.
use anyhow::{Result, anyhow};
use lapce_rpc::{
    RpcMessage,
    core::CoreRpcHandler,
    proxy::{ProxyRpc, ProxyRpcHandler},
    stdio_transport,
};
use std::{io::BufReader, process::Child};
use tracing::debug;

pub fn run_process(
    mut child: Child,
    core_rpc: CoreRpcHandler,
    proxy_rpc: ProxyRpcHandler,
) -> Result<()> {
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("can't find stdin"))?;
    let stdout = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("can't find stdout"))?,
    );
    debug!("process id: {}", child.id());

    let (writer_tx, writer_rx) = crossbeam_channel::unbounded();
    let (reader_tx, reader_rx) = crossbeam_channel::unbounded();
    stdio_transport(stdin, writer_rx, stdout, reader_tx);

    let local_proxy_rpc = proxy_rpc.clone();
    let local_writer_tx = writer_tx.clone();
    let writer_thread = std::thread::Builder::new()
        .name("ProxyRpcHandler".to_owned())
        .spawn(move || {
            for msg in local_proxy_rpc.rx() {
                match msg {
                    ProxyRpc::Request(id, rpc) => {
                        if let Err(err) =
                            local_writer_tx.send(RpcMessage::Request(id, rpc))
                        {
                            tracing::error!("{:?}", err);
                        }
                    }
                    ProxyRpc::Notification(rpc) => {
                        if let Err(err) =
                            local_writer_tx.send(RpcMessage::Notification(rpc))
                        {
                            tracing::error!("{:?}", err);
                        }
                    }
                    ProxyRpc::Shutdown => {
                        if let Err(err) = child.kill() {
                            tracing::error!("{:?}", err);
                        }
                        if let Err(err) = child.wait() {
                            tracing::error!("{:?}", err);
                        }
                        return;
                    }
                }
            }
        })?;

    for msg in reader_rx {
        match msg {
            RpcMessage::Request(id, req) => {
                let writer_tx = writer_tx.clone();
                let core_rpc = core_rpc.clone();
                std::thread::spawn(move || match core_rpc.request(req) {
                    Ok(resp) => {
                        if let Err(err) =
                            writer_tx.send(RpcMessage::Response(id, resp))
                        {
                            tracing::error!("{:?}", err);
                        }
                    }
                    Err(e) => {
                        if let Err(err) = writer_tx.send(RpcMessage::Error(id, e)) {
                            tracing::error!("{:?}", err);
                        }
                    }
                });
            }
            RpcMessage::Notification(n) => {
                core_rpc.notification(n);
            }
            RpcMessage::Response(id, resp) => {
                proxy_rpc.handle_response(id, Ok(resp));
            }
            RpcMessage::Error(id, err) => {
                proxy_rpc.handle_response(id, Err(err));
            }
        }
    }

    proxy_rpc.shutdown();
    writer_thread
        .join()
        .map_err(|_| anyhow!("proxy writer worker panicked"))?;
    Ok(())
}
