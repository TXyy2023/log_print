//! Deliberate byte producers and fault peers; never production log evidence.
use crate::support::*;
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    process::{Command, Stdio},
};
pub fn run(mode: &str, args: &[String]) -> anyhow::Result<()> {
    match mode {
        "gated-core" => crate::gated_core::run(&args[0])?,
        "sleep" => pause(120_000),
        "channels" => {
            let mut b = Vec::new();
            std::io::stdin().read_to_end(&mut b)?;
            assert!(b.is_empty());
            assert!(std::env::var_os("LOG_PRINT_TOKEN").is_none());
            std::io::stdout().write_all(b"A\0\xffwithout-newline")?;
            std::io::stderr().write_all(b"err\0\xfe")?;
        }
        "binary" => {
            assert_eq!(args[0], "001");
            std::io::stdout().write_all(b"out\0\xff")?;
            std::io::stderr().write_all(std::env::var("FLAG")?.as_bytes())?;
            pause(1000)
        }
        "nonzero" => {
            std::io::stderr().write_all(b"failure detail")?;
            std::process::exit(7)
        }
        "tree" => {
            let child = Command::new(std::env::current_exe()?)
                .args(["fixture", "sleep"])
                .stdin(Stdio::null())
                .spawn()?;
            write_json(&args[0], &json!([std::process::id(), child.id()]));
            std::io::stdout().write_all(b"parent ready")?;
            std::io::stdout().flush()?;
            pause(120_000);
        }
        "tmux-source" => {
            println!("BEFORE_ATTACH");
            let mut seen = String::new();
            loop {
                let v = text(&args[0]);
                if v != seen {
                    seen = v;
                    println!("TMUX_{seen}")
                }
                pause(10)
            }
        }
        "failure-peer" => {
            let config: Value = serde_json::from_str(&std::env::var("LOG_PRINT_CONFIG")?)?;
            let path = s(&config["marker"]);
            let previous = text(&path);
            let mut file = fs::File::options().create(true).append(true).open(&path)?;
            writeln!(file, "{}", std::process::id())?;
            drop(file);
            let terminal = config["terminal_once"] == true;
            if terminal && !previous.is_empty() {
                return Ok(());
            }
            let mut rpc = Rpc::connect(
                &std::env::var("LOG_PRINT_CORE")?,
                &std::env::var("LOG_PRINT_PLUGIN")?,
                &std::env::var("LOG_PRINT_TOKEN")?,
                "tcp",
                false,
                "log-print/2",
            )?;
            rpc.call(
                "report",
                json!(
                    { "state" : if terminal { "source_eof" } else { "capturing" },
                    "run_pid" : std::process::id() }
                ),
            );
            if terminal {
                return Ok(());
            }
            loop {
                let msg = rpc.receive()?;
                if msg["type"] == "control" && msg["method"] == "shutdown" {
                    rpc.call(
                        "reply",
                        json!(
                            { "call_id" : msg["call_id"], "result" : { "stopping" : true,
                            "completed" : false }, "error" : null }
                        ),
                    );
                    std::process::exit(7)
                }
            }
        }
        _ => anyhow::bail!("unknown fixture {mode}"),
    }
    Ok(())
}
