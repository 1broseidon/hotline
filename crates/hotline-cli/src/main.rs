//! `hotline`: the desk with no window, and what an operator runs beside it.
//!
//! `hotline serve` is the same desk the app opens — the room, its scheduler,
//! Remote for phones, and the loopback Door — started by systemd on a server
//! with no display and no session bus. Every other subcommand is a client of
//! that running desk's Door, found through `door.json` in the data directory:
//! none of them opens the room itself, so none of them contends for its lock.
//! See `docs/serve.md`.

#[cfg(unix)]
mod door;
#[cfg(unix)]
mod remote;
#[cfg(unix)]
mod serve;

#[cfg(unix)]
use std::path::PathBuf;
use std::process::ExitCode;

#[cfg(unix)]
const USAGE: &str = "\
usage:
  hotline serve --store file|native --listen <IP:port> --public-url <https://host:port>
      [--tls self | --tls-cert <PEM> --tls-key <PEM>] [--data <dir>]
  hotline status [--data <dir>]
  hotline pair [--companion] [--data <dir>]
  hotline devices [--data <dir>]
  hotline revoke <device-id> [--data <dir>]
  hotline wire <command> [--data <dir>] < params.json

`--data` defaults to HOTLINE_DATA_DIR, then the platform's data directory.
`wire` reads the command's params as JSON on stdin, never from arguments,
so a secret in them does not reach shell history or the process list.";

/// The desk with no window is a Unix service: it stops on SIGTERM and keeps
/// its files owner-only. On Windows, the Hotline app is the desk.
#[cfg(not(unix))]
fn main() -> ExitCode {
    eprintln!(
        "hotline: the headless desk runs on Linux and macOS; on Windows, use the Hotline app."
    );
    ExitCode::FAILURE
}

#[cfg(unix)]
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
            let options = match serve_options(&mut args) {
                Ok(options) => options,
                Err(error) => return usage(&error),
            };
            if !args.is_empty() {
                return usage(&format!("serve does not take {}", args.join(" ")));
            }
            serve::run(root, &store, options)
        }
        "pair" if args.is_empty() || args == ["--companion"] => {
            remote::pair(&root, !args.is_empty())
        }
        "devices" if args.is_empty() => remote::devices(&root),
        "revoke" if args.len() == 1 => remote::revoke(&root, &args[0]),
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
#[cfg(unix)]
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

#[cfg(unix)]
fn usage(problem: &str) -> ExitCode {
    if !problem.is_empty() {
        eprintln!("hotline: {problem}\n");
    }
    eprintln!("{USAGE}");
    ExitCode::from(2)
}

#[cfg(unix)]
fn serve_options(args: &mut Vec<String>) -> Result<hotline_core::remote::ServeOptions, String> {
    let listen = take_value(args, "--listen")?
        .ok_or("serve needs --listen with one explicit IP:port")?
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "--listen must be an IP:port, with IPv6 in brackets")?;
    if listen.ip().is_unspecified() || listen.port() == 0 {
        return Err("--listen cannot be a wildcard or port zero".into());
    }
    let public_url =
        take_value(args, "--public-url")?.ok_or("serve needs --public-url for the phone's QR")?;
    let tls = take_value(args, "--tls")?;
    let tls_cert = take_value(args, "--tls-cert")?.map(PathBuf::from);
    let tls_key = take_value(args, "--tls-key")?.map(PathBuf::from);
    if tls.as_deref().is_some_and(|value| value != "self") {
        return Err("--tls only accepts self; use --tls-cert and --tls-key for PEM files".into());
    }
    if tls_cert.is_some() != tls_key.is_some() || (tls.is_some() && tls_cert.is_some()) {
        return Err("supply both --tls-cert and --tls-key, or --tls self, not both".into());
    }
    let options = hotline_core::remote::ServeOptions {
        listen,
        public_url,
        tls_cert,
        tls_key,
    };
    options.validate()?;
    Ok(options)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn options(flags: &[&str]) -> Result<hotline_core::remote::ServeOptions, String> {
        serve_options(&mut flags.iter().map(|s| s.to_string()).collect())
    }
    #[test]
    fn served_network_is_explicit_and_never_wildcard_or_ephemeral() {
        assert!(options(&[]).is_err());
        for address in ["0.0.0.0:9443", "[::]:9443", "127.0.0.1:0", "host:9443"] {
            assert!(
                options(&["--listen", address, "--public-url", "https://desk.example"]).is_err()
            );
        }
        let valid = options(&[
            "--listen",
            "127.0.0.1:9443",
            "--public-url",
            "https://desk.example",
        ])
        .unwrap();
        assert_eq!(valid.listen.port(), 9443);
        assert!(valid.tls_cert.is_none());
    }
    #[test]
    fn public_urls_cannot_smuggle_credentials_or_an_unencrypted_route() {
        for url in [
            "http://desk.example",
            "https://person:secret@desk.example",
            "https://desk.example?token=secret",
            "https://desk.example/#secret",
        ] {
            assert!(options(&["--listen", "127.0.0.1:9443", "--public-url", url]).is_err());
        }
    }
    #[test]
    fn tls_flags_cannot_silently_fall_back() {
        for extra in [
            vec!["--tls", "none"],
            vec!["--tls-cert", "cert"],
            vec!["--tls-key", "key"],
            vec!["--tls", "self", "--tls-cert", "cert", "--tls-key", "key"],
        ] {
            let mut flags = vec![
                "--listen",
                "127.0.0.1:9443",
                "--public-url",
                "https://desk.example",
            ];
            flags.extend(extra);
            assert!(options(&flags).is_err());
        }
    }
}
