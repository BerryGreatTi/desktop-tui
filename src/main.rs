mod terminal_emulation;
mod tui_window;
mod keyboard;
mod desktop;
mod shortcut;
mod utils;
mod args;
mod server;
mod client;
mod protocol;

use std::path::PathBuf;
use std::process::exit;
use crate::desktop::MyDesktop;
use crate::shortcut::parse_shortcut_dir;
use appcui::backend::Type;
use appcui::prelude::{App, Theme};
use appcui::system::Themes;
use clap::Parser;
use crate::args::{Args, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match args.command {
        None => {
            // Backward compat: no subcommand given.
            // Use shortcut_dir positional arg if provided, otherwise default to ".".
            let dir = args.shortcut_dir.unwrap_or_else(|| PathBuf::from("."));
            run_desktop(dir).await?;
        }
        Some(Commands::Run { shortcut_dir }) => {
            run_desktop(shortcut_dir).await?;
        }
        Some(Commands::Serve { shortcut_dir, session }) => {
            server::serve(shortcut_dir, session).await?;
        }
        Some(Commands::Attach { session }) => {
            client::attach(session).await?;
        }
        Some(Commands::List) => {
            client::list_sessions()?;
        }
    }

    exit(0);
}

async fn run_desktop(shortcut_dir: PathBuf) -> anyhow::Result<()> {
    let desktop_shortcuts = parse_shortcut_dir(shortcut_dir)?;
    let theme = Theme::new(Themes::Default);
    let app = App::with_backend(Type::CrossTerm)
        .desktop(MyDesktop::new(desktop_shortcuts))
        .app_bar()
        .theme(theme)
        .color_schema(false)
        .build()?;
    app.run();
    Ok(())
}
