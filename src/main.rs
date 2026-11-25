//! cliphist-picker: a fast clipboard history picker for Hyprland.
//!
//! This is the client: `cliphist-picker [toggle|show|hide]` sends a command to
//! the `cliphist-pickerd` daemon over a unix socket, and starts the daemon if
//! it isn't running. It deliberately doesn't link GTK, so it starts in ~1 ms.

mod ipc;

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode, Stdio};

const USAGE: &str = "usage: cliphist-picker [toggle|show|hide]";

fn main() -> ExitCode {
    let cmd = std::env::args().nth(1).unwrap_or_else(|| "toggle".into());
    match cmd.as_str() {
        "toggle" | "show" | "hide" => client(&cmd),
        "-h" | "--help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("cliphist-picker: unknown command {other:?}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn client(cmd: &str) -> ExitCode {
    if let Ok(mut stream) = UnixStream::connect(ipc::socket_path())
        && stream.write_all(format!("{cmd}\n").as_bytes()).is_ok()
    {
        return ExitCode::SUCCESS;
    }
    if cmd == "hide" {
        return ExitCode::SUCCESS;
    }
    // No daemon (or a stale socket): start one that shows itself right away.
    match spawn_daemon() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cliphist-picker: failed to start cliphist-pickerd: {e}");
            ExitCode::FAILURE
        }
    }
}

fn spawn_daemon() -> std::io::Result<()> {
    let exe = std::env::current_exe()?.with_file_name("cliphist-pickerd");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(ipc::runtime_dir().join("cliphist-picker.log"))?;
    Command::new(exe)
        .arg("--show")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()?;
    Ok(())
}
