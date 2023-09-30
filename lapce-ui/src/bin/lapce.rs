use std::thread;

use lapce_proxy::mainloop;
use lapce_ui::app;

pub fn main() {
    let (writer_tx, writer_rx) = crossbeam_channel::unbounded();
    let (reader_tx, reader_rx) = crossbeam_channel::unbounded();

    thread::spawn(move || mainloop(writer_tx, writer_rx, reader_tx, reader_rx));
    app::launch();
}
