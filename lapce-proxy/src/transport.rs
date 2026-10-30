//! Framed local RPC transport used by an embedded backend thread.
use std::{
    io::BufReader,
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
};

use anyhow::{Result, anyhow};
use lapce_rpc::{
    RpcMessage,
    core::{CoreNotification, CoreRequest, CoreResponse, CoreRpc, CoreRpcHandler},
    proxy::{ProxyNotification, ProxyRequest, ProxyResponse, ProxyRpcHandler},
};

use crate::dispatch::Dispatcher;

type BackendMessage = RpcMessage<CoreRequest, CoreNotification, ProxyResponse>;

/// Own a backend session until the frontend shuts down or closes its pipe.
pub fn serve_stream(stream: UnixStream) -> Result<()> {
    tracing::info!(pid = std::process::id(), "Embedded backend started");
    let core = CoreRpcHandler::new();
    let proxy = ProxyRpcHandler::new();
    let mut dispatcher = Dispatcher::new(core.clone(), proxy.clone());
    let reader = stream.try_clone()?;
    let socket = Arc::new(Mutex::new(stream));
    let event_socket = socket.clone();
    let event_core = core.clone();
    let events = std::thread::Builder::new()
        .name("Lapce backend events".into())
        .spawn(move || {
            for event in event_core.rx() {
                let event: BackendMessage = match event {
                    CoreRpc::Notification(event) => RpcMessage::Notification(*event),
                    CoreRpc::Request(_, request) => match request {},
                    CoreRpc::Shutdown => break,
                };
                if lapce_rpc::stdio::write_msg(
                    &mut *event_socket.lock().unwrap(),
                    event,
                )
                .is_err()
                {
                    break;
                }
            }
            let _ = event_socket.lock().unwrap().shutdown(Shutdown::Both);
        })?;
    let shutdown_core = core.clone();
    let request_proxy = proxy.clone();
    let responses = socket.clone();
    let requests = std::thread::Builder::new()
        .name("Lapce backend requests".into())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            loop {
                let message: RpcMessage<
                    ProxyRequest,
                    ProxyNotification,
                    CoreResponse,
                > = match lapce_rpc::stdio::read_msg(&mut reader) {
                    Ok(Some(message)) => message,
                    Ok(None) => continue,
                    Err(_) => break,
                };
                match message {
                    RpcMessage::Request(id, request) => {
                        let responses = responses.clone();
                        request_proxy.request_async(request, move |result| {
                            let response: BackendMessage = match result {
                                Ok(response) => RpcMessage::Response(id, response),
                                Err(error) => RpcMessage::Error(id, error),
                            };
                            let _ = lapce_rpc::stdio::write_msg(
                                &mut *responses.lock().unwrap(),
                                response,
                            );
                        });
                    }
                    RpcMessage::Notification(notification) => {
                        request_proxy.notification(notification)
                    }
                    RpcMessage::Response(_, response) => match response {},
                    RpcMessage::Error(id, error) => {
                        core.handle_response(id, Err(error))
                    }
                }
            }
            // EOF is also a session shutdown, including language servers and terminals.
            request_proxy.notification(ProxyNotification::Shutdown {});
        })?;
    proxy.mainloop(&mut dispatcher);
    let _ = socket.lock().unwrap().shutdown(Shutdown::Both);
    shutdown_core.shutdown();
    requests
        .join()
        .map_err(|_| anyhow!("backend request worker panicked"))?;
    events
        .join()
        .map_err(|_| anyhow!("backend event worker panicked"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_the_frontend_pipe_stops_the_backend_workers() {
        let (frontend, backend) = UnixStream::pair().unwrap();
        let (done, result) = std::sync::mpsc::channel();
        let worker =
            std::thread::spawn(move || done.send(serve_stream(backend)).unwrap());
        drop(frontend);
        result
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("backend did not stop after EOF")
            .unwrap();
        worker.join().unwrap();
    }
}
