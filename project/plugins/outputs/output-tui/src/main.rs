mod client;
mod render;
mod ui;
use anyhow::{bail, Result};
use clap::Parser;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{CrosstermBackend, TestBackend},
    Terminal,
};
use std::{
    io::{self, IsTerminal},
    time::Duration,
};
#[derive(Parser)]
#[command(
    about = "Persistent terminal display Output. Start through log-print; use log-print tui ID attach to view."
)]
struct Args {
    #[arg(long)]
    attach: Option<String>,
    #[arg(long)]
    snapshot: bool,
    #[arg(long,default_value_t=120,value_parser=clap::value_parser!(u16).range(20..=400))]
    width: u16,
    #[arg(long,default_value_t=36,value_parser=clap::value_parser!(u16).range(8..=150))]
    height: u16,
}
struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(url) = args.attach {
        return viewer(&url, args.snapshot, args.width, args.height).await;
    }
    if args.snapshot {
        bail!("--snapshot requires --attach");
    }
    log_view::server::serve(
        "tui",
        axum::Router::new().route(
            "/",
            axum::routing::get(|| async { "output-tui: attach with log-print tui ID attach\n" }),
        ),
    )
    .await
}
async fn viewer(url: &str, snapshot: bool, width: u16, height: u16) -> Result<()> {
    let connection = client::Connection::new(url)?;
    if snapshot {
        let mut state = client::Snapshot::default();
        connection.refresh(&mut state).await?;
        let mut app = ui::App::new();
        app.accept(state);
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|f| render::draw(f, &mut app))?;
        println!("{}", render::plain(terminal.backend().buffer()));
        return Ok(());
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("attach requires an interactive terminal; use --snapshot for plain text");
    }
    // Validate connection before changing the user's terminal mode.
    let mut initial = client::Snapshot::default();
    connection.refresh(&mut initial).await?;
    enable_raw_mode()?;
    let _screen = Screen;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (commands, mut updates, task) = client::worker(connection);
    let mut app = ui::App::new();
    app.accept(initial);
    let stop = termination();
    tokio::pin!(stop);
    let result: Result<()> = async {
        let mut dirty = true;
        loop {
            if updates.has_changed().unwrap_or(false) {
                let snapshot = updates.borrow_and_update().clone();
                if snapshot.state.is_object() {
                    app.accept(snapshot);
                    dirty = true;
                }
            }
            if dirty {
                terminal.draw(|f| render::draw(f, &mut app))?;
                dirty = false;
            }
            if event::poll(Duration::from_millis(40))? {
                let input = event::read()?;
                dirty = true;
                if let Event::Resize(_, _) = input {
                    terminal.autoresize()?;
                }
                if !app.event(input, &commands)? {
                    break;
                }
            }
            tokio::select! {biased; _=&mut stop=>break, _=tokio::task::yield_now()=>{}}
        }
        Ok(())
    }
    .await;
    task.abort();
    result
}

async fn termination() {
    #[cfg(unix)]
    {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_=signal.recv()=>{},_=tokio::signal::ctrl_c()=>{}};
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
