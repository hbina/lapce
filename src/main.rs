mod lapce_core;
mod lapce_data;
mod lapce_proxy;
mod lapce_rpc;
mod lapce_ui;

use crate::{lapce_proxy::mainloop, lapce_ui::app};

use crossbeam_channel;

pub fn main() {
    let (writer_tx, writer_rx) = crossbeam_channel::unbounded();
    let (reader_tx, reader_rx) = crossbeam_channel::unbounded();

    std::thread::spawn(move || mainloop(writer_tx, writer_rx, reader_tx, reader_rx));
    app::launch();
}
