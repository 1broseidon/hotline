pub mod a11y;
pub mod browser;
pub mod lease;
pub mod serve;
pub mod tools;
pub mod x11;

use std::path::PathBuf;
use std::sync::Arc;

use browser::BrowserManager;
use lease::MachineAccess;

#[derive(Clone, Debug)]
pub struct Config {
    pub addr: String,
    pub token: Option<String>,
    pub home: PathBuf,
    pub display: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            addr: std::env::var("TOAD_COMPUTER_ADDR").unwrap_or_else(|_| "0.0.0.0:8787".to_owned()),
            token: std::env::var("TOAD_COMPUTER_TOKEN")
                .ok()
                .filter(|value| !value.is_empty()),
            home: std::env::var_os("TOAD_COMPUTER_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/home/agent")),
            display: std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_owned()),
        }
    }
}

#[derive(Clone)]
pub struct App {
    pub config: Arc<Config>,
    pub access: MachineAccess,
    pub browser: BrowserManager,
}

impl App {
    pub fn new(config: Config) -> Self {
        let config = Arc::new(config);
        Self {
            access: MachineAccess::new(),
            browser: BrowserManager::new(Arc::clone(&config)),
            config,
        }
    }
}
