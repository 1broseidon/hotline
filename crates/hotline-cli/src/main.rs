//! `hotline`: the desk with no window, and what an operator runs beside it.
//!
//! `hotline serve` is the same desk the app opens — the room, its scheduler,
//! Remote for phones, and the loopback Door — started by systemd on a server
//! with no display and no session bus. Every other subcommand is a client of
//! that running desk's Door, found through `door.json` in the data directory:
//! none of them opens the room itself, so none of them contends for its lock.
//! See `docs/serve.md`.

mod door;
mod serve;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
usage:
  hotline serve --store file|native [--data <dir>]
  hotline status [--data <dir>]
  hotline wire <command> [--data <dir>] < params.json

`--data` defaults to HOTLINE_DATA_DIR, then the platform's data directory.
`wire` reads the command's params as JSON on stdin, never from arguments,
so a secret in them does not reach shell history or the process list.";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let data = match take_value(&mut args, "--data") {
        Ok(data) => data.map(PathBuf::from),
        Err(error) => return usage(&error),
    };
    let root = data.unwrap_or_else(hotline_core::paths::data_root);
    let Some(command) = (!args.is_empty()).then(|| args.remove(0)) else {
        return usage("");
    };
    match command.as_str() {
        "serve" => {
            let store = match take_value(&mut args, "--store") {
                Ok(Some(store)) => store,
                Ok(None) => {
                    return usage(
                        "serve needs --store: file on a server, native where an OS keychain is unlocked",
                    );
                }
                Err(error) => return usage(&error),
            };
            if !args.is_empty() {
                return usage(&format!("serve does not take {}", args.join(" ")));
            }
            serve::run(root, &store)
        }
        "status" if args.is_empty() => door::status(&root),
        "wire" if args.len() == 1 => door::wire(&root, &args[0]),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => usage(&format!(
            "unknown command: {}",
            std::iter::once(command)
                .chain(args)
                .collect::<Vec<_>>()
                .join(" ")
        )),
    }
}

/// Removes `--flag value` or `--flag=value` from `args`.
fn take_value(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    let prefix = format!("{flag}=");
    if let Some(at) = args.iter().position(|arg| arg.starts_with(&prefix)) {
        let value = args.remove(at)[prefix.len()..].to_string();
        return Ok(Some(value));
    }
    let Some(at) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    args.remove(at);
    if at >= args.len() {
        return Err(format!("{flag} needs a value"));
    }
    Ok(Some(args.remove(at)))
}

fn usage(problem: &str) -> ExitCode {
    if !problem.is_empty() {
        eprintln!("hotline: {problem}\n");
    }
    eprintln!("{USAGE}");
    ExitCode::from(2)
}
