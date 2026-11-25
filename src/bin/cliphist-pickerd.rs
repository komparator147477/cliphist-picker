//! cliphist-pickerd: keeps the hidden picker window alive and shows/hides it
//! on commands from `cliphist-picker` over a unix socket.

#[path = "../diff.rs"]
mod diff;
#[path = "../history.rs"]
mod history;
#[path = "../ipc.rs"]
mod ipc;
#[path = "../keys.rs"]
mod keys;
#[path = "../marks.rs"]
mod marks;
#[path = "../thumbs.rs"]
mod thumbs;
#[path = "../ui.rs"]
mod ui;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::ExitCode;

fn main() -> ExitCode {
    let show_now = std::env::args().skip(1).any(|a| a == "--show");
    let path = ipc::socket_path();
    if let Ok(mut stream) = UnixStream::connect(&path) {
        // Already running: just pass the request on.
        if show_now {
            let _ = stream.write_all(b"show\n");
        }
        return ExitCode::SUCCESS;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cliphist-pickerd: binding {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let (tx, rx) = async_channel::unbounded::<String>();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok()
                && tx.send_blocking(line).is_err()
            {
                break;
            }
        }
    });

    let code = ui::run(rx, show_now);
    let _ = std::fs::remove_file(&path);
    if code == gtk::glib::ExitCode::SUCCESS {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
