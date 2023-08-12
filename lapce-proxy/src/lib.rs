#![allow(clippy::manual_clamp)]

pub mod buffer;
pub mod dispatch;
pub mod plugin;
pub mod terminal;
pub mod watcher;

use std::{sync::Arc, thread};

use crossbeam_channel::{Receiver, Sender};
use dispatch::Dispatcher;
use lapce_rpc::{
    core::{CoreNotification, CoreRequest, CoreResponse, CoreRpc, CoreRpcHandler},
    proxy::{ProxyNotification, ProxyRequest, ProxyResponse, ProxyRpcHandler},
    RpcMessage,
};

pub fn mainloop(
    writer_tx: Sender<RpcMessage<CoreRequest, CoreNotification, ProxyResponse>>,
    writer_rx: Receiver<RpcMessage<CoreRequest, CoreNotification, ProxyResponse>>,
    reader_tx: Sender<RpcMessage<ProxyRequest, ProxyNotification, CoreResponse>>,
    reader_rx: Receiver<RpcMessage<ProxyRequest, ProxyNotification, CoreResponse>>,
) {
    let core_rpc = CoreRpcHandler::new();
    let proxy_rpc = ProxyRpcHandler::new();
    let mut dispatcher = Dispatcher::new(core_rpc.clone(), proxy_rpc.clone());

    // let (writer_tx, writer_rx) = crossbeam_channel::unbounded();
    // let (reader_tx, reader_rx) = crossbeam_channel::unbounded();
    // stdio_transport(stdout(), writer_rx, BufReader::new(stdin()), reader_tx);

    let local_core_rpc = core_rpc.clone();
    let local_writer_tx = writer_tx.clone();
    thread::spawn(move || {
        for msg in local_core_rpc.rx() {
            match msg {
                CoreRpc::Request(id, rpc) => {
                    let _ = local_writer_tx.send(RpcMessage::Request(id, rpc));
                }
                CoreRpc::Notification(rpc) => {
                    let _ = local_writer_tx.send(RpcMessage::Notification(*rpc));
                }
                CoreRpc::Shutdown => {
                    return;
                }
            }
        }
    });

    let local_proxy_rpc = proxy_rpc.clone();
    let writer_tx = Arc::new(writer_tx);
    thread::spawn(move || {
        for msg in reader_rx {
            match msg {
                RpcMessage::Request(id, req) => {
                    let writer_tx = writer_tx.clone();
                    local_proxy_rpc.request_async(req, move |result| match result {
                        Ok(resp) => {
                            let _ = writer_tx.send(RpcMessage::Response(id, resp));
                        }
                        Err(e) => {
                            let _ = writer_tx.send(RpcMessage::Error(id, e));
                        }
                    });
                }
                RpcMessage::Notification(n) => {
                    local_proxy_rpc.notification(n);
                }
                RpcMessage::Response(id, resp) => {
                    core_rpc.handle_response(id, Ok(resp));
                }
                RpcMessage::Error(id, err) => {
                    core_rpc.handle_response(id, Err(err));
                }
            }
        }
        local_proxy_rpc.shutdown();
    });

    proxy_rpc.mainloop(&mut dispatcher);
}
