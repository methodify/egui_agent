//! Stdio transport: newline-delimited JSON over stdin/stdout, for apps
//! spawned directly under an agent host.

use crate::hub::CommandHub;

pub(crate) fn serve(hub: CommandHub) {
    std::thread::Builder::new()
        .name("egui_agent-stdio".into())
        .spawn(move || {
            let stdin = std::io::stdin().lock();
            let stdout = std::io::stdout().lock();
            super::serve_connection(&hub, stdin, stdout);
            log::info!("egui_agent: stdio transport reached EOF");
        })
        .expect("failed to spawn stdio transport thread");
}
