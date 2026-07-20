//! Cross-platform local-socket transport via `interprocess`: a named pipe on
//! Windows, a UDS in an abstract/namespaced path elsewhere. Use this on
//! Windows where the `uds` transport isn't available.

use std::io::BufReader;

use interprocess::local_socket::traits::Listener as _;
use interprocess::local_socket::{GenericNamespaced, Listener, ListenerOptions, Stream, ToNsName};
use interprocess::TryClone as _;

use crate::hub::CommandHub;

pub(crate) fn serve(hub: CommandHub, name: String) {
    std::thread::Builder::new()
        .name("egui_agent-pipe".into())
        .spawn(move || {
            let ns_name = match name.clone().to_ns_name::<GenericNamespaced>() {
                Ok(n) => n,
                Err(e) => {
                    log::error!("egui_agent: invalid pipe name {name:?}: {e}");
                    return;
                }
            };
            let listener: Listener = match ListenerOptions::new().name(ns_name).create_sync() {
                Ok(l) => l,
                Err(e) => {
                    log::error!("egui_agent: failed to create local socket {name:?}: {e}");
                    return;
                }
            };
            log::info!("egui_agent: listening on local socket {name:?}");
            loop {
                let Ok(stream) = listener.accept() else {
                    continue;
                };
                let hub = hub.clone();
                let _ = std::thread::Builder::new()
                    .name("egui_agent-pipe-conn".into())
                    .spawn(move || {
                        let Ok(read_half) = stream.try_clone() else {
                            log::warn!("egui_agent: pipe clone failed");
                            return;
                        };
                        let reader = BufReader::new(StreamRead(read_half));
                        super::serve_connection(&hub, reader, StreamWrite(stream));
                    });
            }
        })
        .expect("failed to spawn pipe listener thread");
}

struct StreamRead(Stream);
struct StreamWrite(Stream);

impl std::io::Read for StreamRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(&mut &self.0, buf)
    }
}

impl std::io::Write for StreamWrite {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::io::Write::write(&mut &self.0, buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        std::io::Write::flush(&mut &self.0)
    }
}
