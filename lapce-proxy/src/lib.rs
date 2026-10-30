#![allow(clippy::manual_clamp)]

pub mod buffer;
pub mod cli;
pub mod dispatch;
pub mod plugin;
pub mod terminal;
#[cfg(unix)]
pub mod transport;
pub mod watcher;

use std::{
    io::{BufReader, stdin, stdout},
    sync::Arc,
    thread,
};

use clap::Parser;
use dispatch::Dispatcher;
use lapce_core::meta;
use lapce_rpc::{
    RpcMessage,
    core::{CoreRpc, CoreRpcHandler},
    proxy::ProxyRpcHandler,
    stdio::stdio_transport,
};

#[derive(Parser)]
#[clap(name = "Lapce-proxy")]
#[clap(version = meta::VERSION)]
struct Cli {
    #[clap(short, long, action, hide = true)]
    proxy: bool,

    /// Compatibility option; desktop CLI routing has been removed.
    #[clap(long, hide = true)]
    no_cli_socket: bool,
}

pub fn mainloop() {
    let _cli = Cli::parse();
    let (_reload, _guard) = lapce_services::logging::init("lapce-proxy");

    let core_rpc = CoreRpcHandler::new();
    let proxy_rpc = ProxyRpcHandler::new();
    let mut dispatcher = Dispatcher::new(core_rpc.clone(), proxy_rpc.clone());

    let (writer_tx, writer_rx) = crossbeam_channel::unbounded();
    let (reader_tx, reader_rx) = crossbeam_channel::unbounded();
    stdio_transport(stdout(), writer_rx, BufReader::new(stdin()), reader_tx);

    let local_core_rpc = core_rpc.clone();
    let local_writer_tx = writer_tx.clone();
    thread::spawn(move || {
        for msg in local_core_rpc.rx() {
            match msg {
                CoreRpc::Request(id, rpc) => {
                    if let Err(err) =
                        local_writer_tx.send(RpcMessage::Request(id, rpc))
                    {
                        tracing::error!("{:?}", err);
                    }
                }
                CoreRpc::Notification(rpc) => {
                    if let Err(err) =
                        local_writer_tx.send(RpcMessage::Notification(rpc))
                    {
                        tracing::error!("{:?}", err);
                    }
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
                            if let Err(err) =
                                writer_tx.send(RpcMessage::Response(id, resp))
                            {
                                tracing::error!("{:?}", err);
                            }
                        }
                        Err(e) => {
                            if let Err(err) =
                                writer_tx.send(RpcMessage::Error(id, e))
                            {
                                tracing::error!("{:?}", err);
                            }
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

    if let Err(err) = register_lapce_path() {
        tracing::error!("{:?}", err);
    }

    proxy_rpc.mainloop(&mut dispatcher);
}

pub use lapce_services::environment::register_lapce_path;

pub use lapce_services::http::get_url;
