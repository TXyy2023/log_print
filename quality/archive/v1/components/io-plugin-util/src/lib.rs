//! Shared lifecycle/config helpers for official byte input/output plugins.
use anyhow::{bail, Result};
use log_plugin_sdk::{Client, Control, Event};
use log_proto::Fault;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};

pub struct Context {
    pub client: Client,
    pub events: mpsc::Receiver<Event>,
    pub shutdown: watch::Receiver<bool>,
    pub config: watch::Receiver<Value>,
}

pub async fn connect(
    validate: fn(&Value) -> Result<Value>,
    mutable: &'static [&'static str],
) -> Result<Context> {
    let (client, events, controls) = log_plugin_sdk::connect_env().await?;
    let effective = validate(client.config())?;
    let origins: std::collections::BTreeMap<String, String> = effective
        .as_object()
        .unwrap()
        .keys()
        .map(|key| {
            (
                key.clone(),
                if client.config().get(key).is_some() {
                    "plugin_config".into()
                } else {
                    "built_in_default".into()
                },
            )
        })
        .collect();
    let (config_tx, config) = watch::channel(effective);
    let (shutdown_tx, shutdown) = watch::channel(false);
    let c = client.clone();
    tokio::spawn(async move {
        let result = controls_loop(
            c,
            controls,
            config_tx,
            shutdown_tx.clone(),
            validate,
            mutable,
            origins,
        )
        .await;
        if let Err(e) = result {
            eprintln!("control connection failed: {e:#}");
        }
        let _ = shutdown_tx.send(true);
    });
    Ok(Context {
        client,
        events,
        shutdown,
        config,
    })
}

async fn controls_loop(
    client: Client,
    mut controls: mpsc::Receiver<Control>,
    config: watch::Sender<Value>,
    shutdown: watch::Sender<bool>,
    validate: fn(&Value) -> Result<Value>,
    mutable: &[&str],
    mut origins: std::collections::BTreeMap<String, String>,
) -> Result<()> {
    loop {
        tokio::select! {
            _ = termination() => { let _ = shutdown.send(true); return Ok(()) }
            c = controls.recv() => {
                let Some(c) = c else { return Ok(()) };
                let answer: Result<Value> = match c.method.as_str() {
                    "shutdown" => Ok(json!({"stopping":true})),
                    "config.get" => Ok(json!({"effective":config.borrow().clone(),"origins":origins,"dynamic_fields":mutable,"others":"restart_required"})),
                    "config.patch" => (|| {
                        let patch=c.args.as_object().ok_or_else(||anyhow::anyhow!("patch must be an object"))?;
                        let mut next=config.borrow().clone();
                        for (k,v) in patch { if !mutable.contains(&k.as_str()) { bail!("{k}: restart_required or unknown field") } next[k]=v.clone(); }
                        let next=validate(&next)?;
                        config.send(next.clone())?;
                        for key in patch.keys(){origins.insert(key.clone(),"runtime_override".into());}
                        Ok(json!({"effective":next,"persisted":false}))
                    })(),
                    _ => Err(anyhow::anyhow!("unsupported control {}", c.method)),
                };
                let (result,error)=match answer {Ok(v)=>(v,None), Err(e)=>(Value::Null,Some(Fault{code:"invalid_control".into(),message:e.to_string()}))};
                client.reply_control(c.call_id,result,error).await?;
                if c.method=="shutdown" { let _=shutdown.send(true); return Ok(()) }
            }
        }
    }
}

pub async fn termination() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut sig) = signal(SignalKind::terminate()) {
            tokio::select! {
                received = sig.recv() => {
                    if received.is_none() {
                        std::future::pending::<()>().await;
                    }
                }
                _ = console_interrupt() => {}
            }
            return;
        }
    }
    console_interrupt().await;
}

async fn console_interrupt() {
    wait_console_signal(tokio::signal::ctrl_c()).await;
}

async fn wait_console_signal(signal: impl std::future::Future<Output = std::io::Result<()>>) {
    if let Err(error) = signal.await {
        eprintln!("console signal listener unavailable: {error}; use CLI shutdown");
        // Detached Windows processes may have no console. Registration failure
        // is not a stop request; the independent control receiver stays usable.
        std::future::pending::<()>().await;
    }
}

pub async fn stopped(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return;
        }
    }
}

pub async fn finish(client: &Client, result: &Result<()>) {
    if let Err(e) = result {
        let status = json!({"state":"failed","error":format!("{e:#}")});
        match tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.request("report", status),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => eprintln!("final report failed: {e:#}"),
            Err(e) => eprintln!("final report timed out: {e}"),
        }
    }
    if let Err(e) = result {
        eprintln!("plugin failed: {e:#}");
    }
}

pub fn bounded(name: &str, value: usize, min: usize, max: usize) -> Result<()> {
    if !(min..=max).contains(&value) {
        bail!("{name} must be {min}..={max}")
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::wait_console_signal;
    use std::time::Duration;

    #[tokio::test]
    async fn missing_console_does_not_request_shutdown() {
        let absent = async { Err(std::io::Error::other("detached process has no console")) };
        assert!(
            tokio::time::timeout(Duration::from_millis(20), wait_console_signal(absent))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn actual_console_signal_requests_shutdown() {
        tokio::time::timeout(
            Duration::from_millis(20),
            wait_console_signal(async { Ok(()) }),
        )
        .await
        .expect("a delivered signal should stop the plugin");
    }
}
