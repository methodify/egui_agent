//! Unix domain socket transport: newline-delimited JSON, one thread per
//! connection. Default transport on Linux/macOS.

use std::io::BufReader;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use crate::hub::CommandHub;

/// Bind `path` (removing any stale socket file first) and serve connections
/// on background threads.
pub(crate) fn serve(hub: CommandHub, path: PathBuf) {
    std::thread::Builder::new()
        .name("egui_agent-uds".into())
        .spawn(move || {
            let _ = std::fs::remove_file(&path);
            let listener = match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) => {
                    log::error!("egui_agent: failed to bind UDS {path:?}: {e}");
                    return;
                }
            };
            log::info!("egui_agent: listening on UDS {path:?}");
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let hub = hub.clone();
                let _ = std::thread::Builder::new()
                    .name("egui_agent-uds-conn".into())
                    .spawn(move || {
                        let reader = BufReader::new(match stream.try_clone() {
                            Ok(s) => s,
                            Err(e) => {
                                log::warn!("egui_agent: UDS clone failed: {e}");
                                return;
                            }
                        });
                        super::serve_connection(&hub, reader, stream);
                    });
            }
        })
        .expect("failed to spawn UDS listener thread");
}
