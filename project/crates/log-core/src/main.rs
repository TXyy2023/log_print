use anyhow::{Context, Result};
use log_core::Core;
use log_proto::RuntimeConfig;
use std::io::Read;
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let value = |key: &str| -> Result<&str> {
        let i = args
            .iter()
            .position(|x| x == key)
            .with_context(|| format!("missing {key}"))?;
        args.get(i + 1)
            .map(String::as_str)
            .context("missing argument")
    };
    let runtime: RuntimeConfig =
        serde_json::from_slice(&std::fs::read(value("--runtime-config")?)?)?;
    let core = Core::new(runtime)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let ready = serde_json::to_vec(
        &serde_json::json!({"address":listener.local_addr()?.to_string(),"pid":std::process::id()}),
    )?;
    let ready_path = value("--ready-file")?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        options.open(ready_path)?.write_all(&ready)?;
    }
    let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
    std::thread::spawn(move || {
        let mut byte = [0u8; 1];
        while matches!(std::io::stdin().read(&mut byte),Ok(n) if n>0) {}
        let _ = stop_tx.send(());
    });
    loop {
        tokio::select! {
            _=&mut stop_rx=>break,
        _=async {if tokio::signal::ctrl_c().await.is_err(){std::future::pending::<()>().await}}=>break,
            incoming=listener.accept()=>{let (socket,_)=incoming?;let core=core.clone();tokio::spawn(async move{if let Err(e)=core.connection(socket).await{eprintln!("Core connection: {e:#}");}});}
        }
    }
    let _ = std::fs::remove_file(ready_path);
    Ok(())
}
