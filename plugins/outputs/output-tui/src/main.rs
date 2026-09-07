use anyhow::{bail, Context, Result};
use clap::Parser;
use crossterm::{
    cursor::{Hide, Show},
    event::{self, Event as TerminalEvent, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use log_plot::{lock, OutputConfig, PlotHub, Snapshot};
use log_plugin_sdk::{Client, Event};
use log_proto::Fault;
use ratatui::{
    backend::{Backend, ClearType, CrosstermBackend, WindowSize},
    buffer::Cell,
    layout::{Constraint, Direction, Layout, Position, Rect, Size},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Wrap},
    Terminal, TerminalOptions, Viewport,
};
use serde_json::{json, Value};
use std::{
    io::{IsTerminal, Write},
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[derive(Parser)]
#[command(about = "Independent log_print terminal plotting Output; start through log-print")]
struct Args {
    #[arg(long)]
    headless: bool,
    /// Write to an explicitly selected terminal. On Unix this is display-only; control through CLI.
    #[arg(long)]
    tty: Option<String>,
    #[arg(long)]
    session: Option<String>,
}
struct Screen {
    terminal: Terminal<TtyBackend>,
    raw: bool,
    size: (u16, u16),
    #[cfg(unix)]
    file: Option<std::fs::File>,
}
// CrosstermBackend::size queries the process controlling terminal/stdout, even when
// its writer is another TTY. Ratatui also invokes it while clearing a fixed viewport
// on resize. Keep that lookup tied to the terminal selected by Screen instead.
struct TtyBackend {
    inner: CrosstermBackend<Box<dyn Write + Send>>,
    size: Size,
}
impl Write for TtyBackend {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.inner.write(bytes)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Write::flush(&mut self.inner)
    }
}
impl Backend for TtyBackend {
    type Error = std::io::Error;
    fn draw<'a, I>(&mut self, content: I) -> std::io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.inner.draw(content)
    }
    fn hide_cursor(&mut self) -> std::io::Result<()> {
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> std::io::Result<()> {
        self.inner.show_cursor()
    }
    fn get_cursor_position(&mut self) -> std::io::Result<Position> {
        // The full-screen fixed viewport never needs a terminal response query.
        // Do not accidentally read from an unrelated controlling terminal.
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "TTY cursor querying is not used by this fixed viewport",
        ))
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> std::io::Result<()> {
        self.inner.set_cursor_position(position)
    }
    fn clear(&mut self) -> std::io::Result<()> {
        self.inner.clear()
    }
    fn clear_region(&mut self, clear_type: ClearType) -> std::io::Result<()> {
        self.inner.clear_region(clear_type)
    }
    fn size(&self) -> std::io::Result<Size> {
        Ok(self.size)
    }
    fn window_size(&mut self) -> std::io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size,
            pixels: Size::new(0, 0),
        })
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Write::flush(self)
    }
}
impl Screen {
    fn open(path: Option<&str>) -> Result<Self> {
        #[cfg(unix)]
        let file = path
            .map(|p| std::fs::OpenOptions::new().read(true).write(true).open(p))
            .transpose()
            .context("open requested TTY")?;
        #[cfg(unix)]
        if file.as_ref().is_some_and(|file| !file.is_terminal()) {
            bail!("--tty must name a terminal, not a regular file");
        }
        #[cfg(not(unix))]
        if path.is_some() {
            bail!("explicit tty paths are supported on Unix; use a foreground terminal on this platform");
        }
        let raw = path.is_none();
        if raw && !std::io::stdout().is_terminal() {
            bail!("stdout is not a terminal; configure tty with the result of `tty`, or use headless=true for diagnostics");
        }
        #[cfg(unix)]
        let writer: Box<dyn Write + Send> = match &file {
            Some(file) => Box::new(file.try_clone()?),
            None => Box::new(std::io::stdout()),
        };
        #[cfg(not(unix))]
        let writer: Box<dyn Write + Send> = Box::new(std::io::stdout());
        let size = terminal_size(
            #[cfg(unix)]
            file.as_ref(),
        );
        let terminal = Terminal::with_options(
            TtyBackend {
                inner: CrosstermBackend::new(writer),
                size: Size::new(size.0, size.1),
            },
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, size.0, size.1)),
            },
        )?;
        let mut screen = Self {
            terminal,
            raw: false,
            size,
            #[cfg(unix)]
            file,
        };
        if raw {
            enable_raw_mode()?;
            screen.raw = true;
        }
        screen
            .terminal
            .backend_mut()
            .execute(EnterAlternateScreen)?
            .execute(Hide)?;
        Ok(screen)
    }
    fn resize(&mut self) -> Result<()> {
        let (w, h) = terminal_size(
            #[cfg(unix)]
            self.file.as_ref(),
        );
        if self.size != (w, h) {
            self.terminal.backend_mut().size = Size::new(w, h);
            self.terminal.resize(Rect::new(0, 0, w, h))?;
            self.size = (w, h);
        }
        Ok(())
    }
}
impl Drop for Screen {
    fn drop(&mut self) {
        if self.raw {
            let _ = disable_raw_mode();
        }
        let _ = self
            .terminal
            .backend_mut()
            .execute(Show)
            .and_then(|w| w.execute(LeaveAlternateScreen));
        let _ = Write::flush(self.terminal.backend_mut());
    }
}
fn terminal_size(#[cfg(unix)] file: Option<&std::fs::File>) -> (u16, u16) {
    #[cfg(unix)]
    if let Some(file) = file {
        use std::os::fd::AsRawFd;
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(file.as_raw_fd(), libc::TIOCGWINSZ, &mut size) } == 0
            && size.ws_col > 0
            && size.ws_row > 0
        {
            return (size.ws_col, size.ws_row);
        }
    }
    crossterm::terminal::size().unwrap_or((100, 30))
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let (client, mut events, mut controls) = log_plugin_sdk::connect_env().await?;
    let mut config = OutputConfig::from_value(client.config())?;
    if args.headless {
        config.headless = true;
    }
    if args.tty.is_some() {
        config.tty = args.tty;
    }
    if args.session.is_some() {
        config.session = args.session;
    }
    let hub = PlotHub::shared(config.clone())?;
    let mut selected = config
        .session
        .clone()
        .unwrap_or_else(|| lock(&hub).unwrap().ids()[0].clone());
    lock(&hub)?.snapshot(&selected)?;
    // Fail before reporting ready when no real terminal was provided.
    let mut screen = if config.headless {
        None
    } else {
        Some(Screen::open(config.tty.as_deref())?)
    };
    for stream in &config.streams {
        client.subscribe(stream, config.from).await?;
    }
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let (selection_tx, mut selection_rx) = watch::channel(selected.clone());
    let ingest_hub = hub.clone();
    let ingest_stop = stop_tx.clone();
    let data_task = tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            if let Ok(mut h) = lock(&ingest_hub) {
                match event {
                    Event::Record(record) => h.ingest(&record),
                    Event::Gap {
                        stream,
                        epoch,
                        from,
                        to,
                        reason,
                    } => h.gap(&stream, &epoch, from, to, &reason),
                    Event::Disconnected { stream, reason } => h.disconnected(&stream, &reason),
                }
            } else {
                break;
            }
        }
        let _ = ingest_stop.send(true);
    });
    let control_hub = hub.clone();
    let control_client = client.clone();
    let control_stop = stop_tx.clone();
    let control_task = tokio::spawn(async move {
        while let Some(call) = controls.recv().await {
            if call.method == "shutdown" {
                let _ = control_client
                    .reply_control(call.call_id, json!({"stopping":true}), None)
                    .await;
                let _ = control_stop.send(true);
                break;
            }
            let result = if call.method == "session.select" {
                let id = call.args.get("id").and_then(Value::as_str).unwrap_or("");
                match lock(&control_hub).and_then(|h| h.snapshot(id)) {
                    Ok(s) => {
                        let _ = selection_tx.send(id.to_owned());
                        Ok(json!({"id":id,"revision":s.revision,"selected":true}))
                    }
                    Err(e) => Err(e),
                }
            } else {
                log_plot::control(&control_hub, &call.method, call.args).await
            };
            reply(&control_client, call.call_id, result).await;
        }
        let _ = control_stop.send(true);
    });
    let session_ids = lock(&hub)?.ids();
    client.request("report",json!({"kind":"tui","ready":true,"headless":config.headless,"tty":config.tty,"session":selected,"sessions":session_ids})).await?;
    let mut last_diagnostic = Instant::now() - Duration::from_secs(2);
    let mut quit = false;
    while !quit && !*stop_rx.borrow() {
        if selection_rx.has_changed().unwrap_or(false) {
            selected = selection_rx.borrow_and_update().clone();
        }
        let snapshot = lock(&hub)?.snapshot(&selected)?;
        if let Some(screen) = &mut screen {
            screen.resize()?;
            screen.terminal.draw(|frame| draw(frame, &snapshot))?;
            if screen.raw {
                while event::poll(Duration::ZERO)? {
                    if let TerminalEvent::Key(key) = event::read()? {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }
                        match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => quit = true,
                            KeyCode::Tab => {
                                let ids = lock(&hub)?.ids();
                                let i = ids.iter().position(|id| id == &selected).unwrap_or(0);
                                selected = ids[(i + 1) % ids.len()].clone();
                            }
                            _ => {}
                        }
                    }
                }
            }
        } else if last_diagnostic.elapsed() >= Duration::from_secs(1) {
            eprintln!("tui headless: session={} revision={} generation={} points={} (no terminal visual acceptance)",snapshot.id,snapshot.revision,snapshot.generation,snapshot.series.iter().map(|s|s.retained_points).sum::<usize>());
            last_diagnostic = Instant::now();
        }
        tokio::select! {_=tokio::time::sleep(Duration::from_millis(snapshot.refresh_ms))=>{},_=stop_rx.changed()=>{},_=shutdown_signal()=>{quit=true;}}
    }
    drop(screen);
    data_task.abort();
    control_task.abort();
    let _ = client
        .request(
            "report",
            json!({"kind":"tui","ready":false,"state":"stopped"}),
        )
        .await;
    Ok(())
}
async fn shutdown_signal() {
    if tokio::signal::ctrl_c().await.is_err() {
        // A detached Windows process may not have a console signal source.
        // Shutdown remains available through the supervisor control channel.
        std::future::pending::<()>().await;
    }
}
async fn reply(client: &Client, id: u64, result: Result<Value>) {
    let (value, error) = match result {
        Ok(v) => (v, None),
        Err(e) => (
            Value::Null,
            Some(Fault {
                code: if e.to_string().starts_with("revision_conflict") {
                    "revision_conflict"
                } else {
                    "invalid_control"
                }
                .into(),
                message: e.to_string(),
            }),
        ),
    };
    if let Err(e) = client.reply_control(id, value, error).await {
        eprintln!("TUI control reply: {e}");
    }
}
fn color(hex: &str) -> Color {
    Color::Rgb(
        u8::from_str_radix(&hex[1..3], 16).unwrap_or(200),
        u8::from_str_radix(&hex[3..5], 16).unwrap_or(200),
        u8::from_str_radix(&hex[5..7], 16).unwrap_or(200),
    )
}
fn draw(frame: &mut ratatui::Frame, s: &Snapshot) {
    let dark = s.theme == "dark";
    let bg = if dark {
        Color::Rgb(20, 27, 39)
    } else {
        Color::Rgb(250, 251, 253)
    };
    let fg = if dark {
        Color::Rgb(222, 229, 238)
    } else {
        Color::Rgb(30, 40, 55)
    };
    frame.render_widget(
        Block::default().style(Style::default().bg(bg).fg(fg)),
        frame.area(),
    );
    if frame.area().width < 40 || frame.area().height < 12 {
        frame.render_widget(
            Paragraph::new(
                "Terminal too small. Resize to at least 40 × 12. Data collection continues.",
            )
            .wrap(Wrap { trim: true }),
            frame.area(),
        );
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(6),
            Constraint::Length(4),
        ])
        .split(frame.area());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    " LOG_PRINT ",
                    Style::default()
                        .fg(Color::Rgb(231, 111, 81))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "{}  ·  {}",
                    s.title,
                    if s.live_source_status
                        .values()
                        .any(|source| source.disconnected)
                    {
                        "SOURCE INTERRUPTED"
                    } else if s.paused {
                        "PAUSED VIEW"
                    } else {
                        "LIVE"
                    }
                )),
            ]),
            Line::raw(format!(
                " Session {}   revision {}   window {:.1}s   refresh {}ms",
                s.id, s.revision, s.window_secs, s.refresh_ms
            )),
        ]),
        rows[0],
    );
    let offset = s.x_range[0];
    type ChartSegment = (String, Color, Vec<(f64, f64)>);
    let mut segments: Vec<ChartSegment> = Vec::new();
    for series in &s.series {
        let mut first = true;
        for segment in series.data.split(|(_, y)| y.is_none()) {
            let data = segment
                .iter()
                .filter_map(|(x, y)| y.map(|y| (*x - offset, y)))
                .collect::<Vec<_>>();
            if data.is_empty() {
                continue;
            }
            segments.push((
                if first {
                    series.name.clone()
                } else {
                    String::new()
                },
                color(&series.color),
                data,
            ));
            first = false;
        }
    }
    let datasets = segments
        .iter()
        .map(|(name, color, data)| {
            let dataset = Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(data);
            if name.is_empty() {
                dataset
            } else {
                dataset.name(name.as_str())
            }
        })
        .collect::<Vec<_>>();
    let duration = s.x_range[1] - offset;
    let xlabels = vec![
        Span::raw(format!("-{duration:.1}s")),
        Span::raw(format!("-{:.1}s", duration / 2.0)),
        Span::raw("latest"),
    ];
    let ylabels = vec![
        Span::raw(format!("{:.3}", s.y_range[0])),
        Span::raw(format!("{:.3}", (s.y_range[0] + s.y_range[1]) / 2.0)),
        Span::raw(format!("{:.3}", s.y_range[1])),
    ];
    let chart = Chart::new(datasets)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Signals ")
                .border_style(Style::default().fg(Color::Rgb(80, 97, 119))),
        )
        .x_axis(
            Axis::default()
                .title("Core observation time")
                .bounds([0.0, duration])
                .labels(xlabels)
                .style(Style::default().fg(fg)),
        )
        .y_axis(
            Axis::default()
                .title("Value")
                .bounds(s.y_range)
                .labels(ylabels)
                .style(Style::default().fg(fg)),
        );
    frame.render_widget(chart, rows[1]);
    let totals = s
        .series
        .iter()
        .map(|x| {
            format!(
                "{}: {} pts / {} invalid",
                x.name, x.retained_points, x.invalid
            )
        })
        .collect::<Vec<_>>()
        .join("  ·  ");
    let hint = if s.series.iter().all(|series| series.data.is_empty()) {
        "Waiting for complete newline-terminated numeric records. "
    } else {
        ""
    };
    frame.render_widget(Paragraph::new(format!("{totals}\n{hint}Gaps remain breaks; view may reduce points.\nTab: session  q: close  ·  CLI session set/export controls this running plugin")).wrap(Wrap{trim:true}),rows[2]);
}
