use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Shortcut directory (backward compat: used when no subcommand is given)
    #[arg(default_value = None)]
    pub shortcut_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run desktop-tui directly (default when no subcommand)
    Run {
        #[arg(default_value = ".")]
        shortcut_dir: PathBuf,
    },
    /// Start desktop-tui as a daemon with session support
    Serve {
        #[arg(default_value = ".")]
        shortcut_dir: PathBuf,
        /// Session name
        #[arg(long, default_value = "default")]
        session: String,
    },
    /// Attach to a running session
    Attach {
        /// Session name
        #[arg(default_value = "default")]
        session: String,
    },
    /// List active sessions
    List,
}
