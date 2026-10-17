//! Unix-domain socket transport for separately launched read-only frontends.
use crate::shared::SharedFrontend;
use anyhow::Result;
use lapce_rpc::{
    RpcMessage,
    core::{CoreNotification, CoreRequest, CoreRpcHandler},
    proxy::{ProxyMessage, ProxyResponse, ProxyRpc, ProxyRpcHandler},
};
use std::{
    io::BufReader,
    net::Shutdown,
    os::unix::net::UnixStream,
    path::Path,
    sync::{Arc, Mutex},
};

type BackendMessage = RpcMessage<CoreRequest, CoreNotification, ProxyResponse>;
fn write(
    stream: &Arc<Mutex<UnixStream>>,
    message: BackendMessage,
) -> std::io::Result<()> {
    lapce_rpc::stdio::write_msg(&mut *stream.lock().unwrap(), message)
}

/// Serve one attached frontend. EOF releases its leases without stopping the host.
pub fn serve_frontend(
    mut frontend: SharedFrontend,
    stream: UnixStream,
) -> Result<()> {
    let reader = stream.try_clone()?;
    let socket = Arc::new(Mutex::new(stream));
    let (_, empty) = std::sync::mpsc::channel();
    let events = std::mem::replace(&mut frontend.events, empty);
    let event_socket = socket.clone();
    let event_worker = std::thread::Builder::new()
        .name("Shared socket events".into())
        .spawn(move || {
            for event in events {
                if write(&event_socket, RpcMessage::Notification(event)).is_err() {
                    break;
                }
            }
            let _ = event_socket.lock().unwrap().shutdown(Shutdown::Both);
        })?;
    let mut reader = BufReader::new(reader);
    loop {
        let message: ProxyMessage = match lapce_rpc::stdio::read_msg(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => continue,
            Err(_) => break,
        };
        match message {
            RpcMessage::Request(id, request) => {
                let writer = socket.clone();
                frontend.proxy_rpc.request_async(request, move |response| {
                    let message = match response {
                        Ok(response) => RpcMessage::Response(id, response),
                        Err(error) => RpcMessage::Error(id, error),
                    };
                    if write(&writer, message).is_err() {
                        let _ = writer.lock().unwrap().shutdown(Shutdown::Both);
                    }
                });
            }
            RpcMessage::Notification(notification) => {
                frontend.proxy_rpc.notification(notification)
            }
            _ => break,
        }
    }
    frontend.proxy_rpc.shutdown();
    let _ = socket.lock().unwrap().shutdown(Shutdown::Both);
    event_worker
        .join()
        .map_err(|_| anyhow::anyhow!("socket event worker panicked"))?;
    Ok(())
}

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
