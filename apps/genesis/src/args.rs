use std::fmt::Display;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, ValueEnum, Default)]
pub enum Environment {
    #[default]
    Development,
    Production,
}

impl From<String> for Environment {
    fn from(value: String) -> Self {
        match value.as_str() {
            "development" => Environment::Development,
            "production" => Environment::Production,
            _ => Environment::Development,
        }
    }
}

impl Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Environment::Development => write!(f, "development"),
            Environment::Production => write!(f, "production"),
        }
    }
}

#[derive(Debug, Clone, Parser)]
#[command(about, version)]
pub struct Args {
    #[command(flatten)]
    pub(crate) bus: BusArgs,
}

#[derive(clap::Args, Debug, Clone)]
pub struct BusArgs {
    #[arg(
        long = "iggy-url",
        env = "IGGY_URL",
        default_value = "iggy://autharie:autharie@localhost:8090",
        name = "Iggy URL",
        help = "The Iggy connection string (e.g. iggy://user:password@host:port)"
    )]
    pub iggy_url: String,

    #[arg(
        long = "iggy-consumer",
        env = "IGGY_CONSUMER",
        default_value = "genesis",
        name = "Iggy consumer",
        help = "The name Genesis reads the actions topic under; the broker keeps its offset by this name"
    )]
    pub iggy_consumer: String,
}

impl Default for BusArgs {
    fn default() -> Self {
        Self {
            iggy_url: "iggy://autharie:autharie@localhost:8090".into(),
            iggy_consumer: "genesis".into(),
        }
    }
}
