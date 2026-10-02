//! Test-only real PTY/ConPTY driver. JSON lines in; parsed terminal screens out.
use anyhow::{bail, Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde_json::{json, Value};
use std::{
    io::{self, BufRead, Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let pair = native_pty_system().openpty(PtySize {
        rows: 36,
        cols: 140,
        pixel_width: 1120,
        pixel_height: 576,
    })?;
    let mut command = CommandBuilder::new(args.get(1).context("binary")?);
    command.args(["--attach", args.get(2).context("URL")?]);
    command.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(command)?;
    drop(pair.slave);
    let parser = Arc::new(Mutex::new(vt100::Parser::new(36, 140, 100)));
    let raw = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut reader = pair.master.try_clone_reader()?;
    let p = parser.clone();
    let output = raw.clone();
    std::thread::spawn(move || {
        let mut bytes = [0; 8192];
        while let Ok(n) = reader.read(&mut bytes) {
            if n == 0 {
                break;
            }
            p.lock().unwrap().process(&bytes[..n]);
            let mut raw = output.lock().unwrap();
            if raw.len() + n > 1024 * 1024 {
                let excess = raw.len() + n - 1024 * 1024;
                raw.drain(..excess);
            }
            raw.extend_from_slice(&bytes[..n]);
        }
    });
    let mut writer = pair.master.take_writer()?;
    println!("{}", json!({"ready":true}));
    io::stdout().flush()?;
    let result = (|| -> Result<()> {
        for line in io::stdin().lock().lines() {
            let request: Value = serde_json::from_str(&line?)?;
            if let Some(size) = request["resize"].as_array() {
                let cols = size[0].as_u64().context("cols")? as u16;
                let rows = size[1].as_u64().context("rows")? as u16;
                parser.lock().unwrap().screen_mut().set_size(rows, cols);
                pair.master.resize(PtySize {
                    cols,
                    rows,
                    pixel_width: cols.saturating_mul(8),
                    pixel_height: rows.saturating_mul(16),
                })?;
            }
            if let Some(s) = request["send"].as_str() {
                writer.write_all(s.as_bytes())?;
                writer.flush()?;
            }
            let finish = request["finish"] == true;
            let expected = request["wait"].as_str();
            let absent = request["absent"].as_str();
            let deadline = Instant::now()
                + Duration::from_millis(request["timeout_ms"].as_u64().unwrap_or(8000));
            let mut exit = None;
            loop {
                let found =
                    expected.is_none_or(|s| parser.lock().unwrap().screen().contents().contains(s));
                let gone =
                    absent.is_none_or(|s| !parser.lock().unwrap().screen().contents().contains(s));
                if finish {
                    exit = child.try_wait()?;
                }
                if found && gone && (!finish || exit.is_some()) {
                    break;
                }
                if Instant::now() > deadline {
                    bail!(
                        "terminal timeout: {request}\n{}",
                        parser.lock().unwrap().screen().contents()
                    );
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            // Allow the PTY reader to consume trailing terminal restoration bytes.
            if finish {
                std::thread::sleep(Duration::from_millis(150));
            }
            let parser = parser.lock().unwrap();
            let raw = raw.lock().unwrap();
            println!(
                "{}",
                json!({"screen":parser.screen().contents(),"alternate":parser.screen().alternate_screen(),"exit":exit.as_ref().map(|e|e.exit_code()),"restored":raw.windows(8).any(|w|w==b"\x1b[?1049l")})
            );
            io::stdout().flush()?;
            if finish {
                break;
            }
        }
        Ok(())
    })();
    if child.try_wait()?.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
