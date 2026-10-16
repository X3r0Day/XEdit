use std::io::{self, Write};
use std::process::ExitCode;

use xedit_core::Error;
use xedit_host::desk::{Desk, Reply};
use xedit_host::policy::{Capability, Policy};
use xedit_xtl::request::Request;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("-V") | Some("--version") => {
            println!("xedit {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("list") => show(desk().handle(Request::ListProcesses)),
        Some("regions") => {
            let Some(pid) = args.get(1).and_then(|s| s.parse::<u32>().ok()) else {
                eprintln!("usage: xedit regions <pid>");
                return ExitCode::FAILURE;
            };
            show(desk().handle(Request::Regions { pid }))
        }
        _ => {
            eprintln!("usage: xedit <list | regions <pid> | --version>");
            ExitCode::FAILURE
        }
    }
}

fn desk() -> Desk {
    Desk::new(Policy::none().allow(Capability::ReadMemory))
}

fn show(reply: Result<Reply, Error>) -> ExitCode {
    match reply {
        Ok(Reply::Processes(procs)) => emit(procs.iter().map(|p| p.to_string())),
        Ok(Reply::Regions(regions)) => emit(regions.iter().map(|r| r.to_string())),
        Ok(Reply::Ok) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn emit(lines: impl Iterator<Item = String>) -> ExitCode {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in lines {
        // `xedit list | head` closes the pipe on us, which is not an error
        if let Err(e) = writeln!(out, "{line}") {
            if e.kind() == io::ErrorKind::BrokenPipe {
                return ExitCode::SUCCESS;
            }
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}
