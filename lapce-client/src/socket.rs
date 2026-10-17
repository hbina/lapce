//! Unix-domain socket transport for separately launched read-only frontends.
use anyhow::Result;
use lapce_rpc::{
    RpcMessage,
    core::{CoreNotification, CoreRequest, CoreRpcHandler},
    proxy::{ProxyMessage, ProxyResponse, ProxyRpc, ProxyRpcHandler},
};
use std::{io::BufReader, net::Shutdown, os::unix::net::UnixStream, path::Path};

type BackendMessage = RpcMessage<CoreRequest, CoreNotification, ProxyResponse>;
pub fn run_client(
    path: &Path,
    core: CoreRpcHandler,
    proxy: ProxyRpcHandler,
) -> Result<()> {
    let stream = UnixStream::connect(path)?;
    let writer_stream = stream.try_clone()?;
    let shutdown_stream = stream.try_clone()?;
    let writer_proxy = proxy.clone();
    let writer = std::thread::Builder::new()
        .name("Shared socket requests".into())
        .spawn(move || {
            let mut stream = writer_stream;
            for message in writer_proxy.rx() {
                let message: ProxyMessage = match message {
                    ProxyRpc::Request(id, request) => {
                        RpcMessage::Request(id, request)
                    }
                    ProxyRpc::Notification(notification) => {
                        RpcMessage::Notification(notification)
                    }
                    ProxyRpc::Shutdown => break,
                };
                if lapce_rpc::stdio::write_msg(&mut stream, message).is_err() {
                    break;
                }
            }
            let _ = stream.shutdown(Shutdown::Both);
        })?;
    let mut reader = BufReader::new(stream);
    loop {
        let message: BackendMessage = match lapce_rpc::stdio::read_msg(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(_) => break,
        };
        match message {
            RpcMessage::Notification(event) => core.notification(event),
            RpcMessage::Response(id, response) => {
                proxy.handle_response(id, Ok(response))
            }
            RpcMessage::Error(id, error) => proxy.handle_response(id, Err(error)),
            RpcMessage::Request(_, request) => match request {},
        }
    }
    proxy.disconnect();
    proxy.shutdown();
    let _ = shutdown_stream.shutdown(Shutdown::Both);
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("socket request worker panicked"))?;
    Ok(())
}
