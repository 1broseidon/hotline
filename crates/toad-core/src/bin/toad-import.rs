//! Import an existing Toad data directory into a new one, without a desk.
//!
//! `toad-import <from> <to>` reads the previous Toad's layout at `<from>`
//! and writes this tree's streams and vault at `<to>`. The source is never
//! written.

use std::path::PathBuf;
use std::process::ExitCode;
use toad_core::import;
use toad_core::log::Log;
use toad_core::vault::Vault;

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(from) = args.next() else {
        return usage();
    };
    let Some(to) = args.next() else {
        return usage();
    };
    if args.next().is_some() {
        return usage();
    }

    if let Err(error) = run(PathBuf::from(from), PathBuf::from(to)) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn run(from: PathBuf, to: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let log = Log::open(&to);
    let vault = Vault::open(&to, log.clone())?;
    let report = import::import(&from, &log, &vault)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn usage() -> ExitCode {
    eprintln!("usage: toad-import <from> <to>");
    ExitCode::from(2)
}
