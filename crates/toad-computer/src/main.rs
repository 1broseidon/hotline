use toad_computer::{Config, serve};

fn usage() -> &'static str {
    "Usage: toad-computer serve [--addr ADDRESS] [--token TOKEN] [--home PATH] [--display DISPLAY]"
}

fn parse() -> Result<Config, String> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => {}
        Some("--help" | "-h") | None => return Err(usage().to_owned()),
        Some(command) => return Err(format!("unknown subcommand {command:?}\n{}", usage())),
    }

    let mut config = Config::from_env();
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            return Err(usage().to_owned());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{flag} requires a value"))?;
        match flag.as_str() {
            "--addr" => config.addr = value,
            "--token" => config.token = (!value.is_empty()).then_some(value),
            "--home" => config.home = value.into(),
            "--display" => config.display = value,
            _ => return Err(format!("unknown flag {flag:?}\n{}", usage())),
        }
    }
    Ok(config)
}

#[tokio::main]
async fn main() {
    let config = match parse() {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    if let Err(error) = serve::run(config).await {
        eprintln!("toad-computer: {error}");
        std::process::exit(1);
    }
}
