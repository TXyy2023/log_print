//! Only transport-independent static configuration and cooperative stop primitives.
use crate::{Client, Control, Event};
use anyhow::{bail, Result};
use log_proto::Fault;
use serde_json::{json, Value};
use tokio::sync::{mpsc, watch};
struct LifecycleTask(tokio::task::AbortHandle);
impl Drop for LifecycleTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}
pub struct Context {
    pub client: Client,
    pub events: mpsc::Receiver<Event>,
    pub shutdown: watch::Receiver<bool>,
    pub config: Value,
    failure: watch::Receiver<Option<String>>,
    _controller: LifecycleTask,
}
impl Context {
    /// Distinguishes requested shutdown from a broken control connection.
    pub fn shutdown_result(&self) -> Result<()> {
        if let Some(error) = self.failure.borrow().as_ref() {
            bail!("{error}");
        }
        Ok(())
    }
}
pub async fn connect(validate: fn(&Value) -> Result<Value>) -> Result<Context> {
    let (client, events, mut controls) = crate::connect_env().await?;
    let config = validate(client.config())?;
    let (stop, shutdown) = watch::channel(false);
    let (failed, failure) = watch::channel(None);
    let task_client = client.clone();
    let snapshot = config.clone();
    let controller = tokio::spawn(async move {
        let result=async {
            loop {
                let control=tokio::select! {_=termination()=>break,control=controls.recv()=>match control {Some(c)=>c,None=>bail!("Core control connection closed")}};
                let (result,error)=control_answer(&control,&snapshot);
                task_client.reply_control(control.call_id,result,error).await?;
                if control.method=="shutdown" {break;}
            } Ok::<(),anyhow::Error>(())
        }.await;
        if let Err(e) = result {
            failed.send_replace(Some(format!("control connection failed: {e:#}")));
            eprintln!("control connection failed: {e:#}");
        }
        stop.send_replace(true);
    });
    Ok(Context {
        client,
        events,
        shutdown,
        config,
        failure,
        _controller: LifecycleTask(controller.abort_handle()),
    })
}
fn control_answer(control: &Control, snapshot: &Value) -> (Value, Option<Fault>) {
    match control.method.as_str() {
        "shutdown" => (json!({"stopping":true,"completed":false}), None),
        "config.get" => (
            json!({"effective":snapshot,"dynamic_fields":[],"restart_required":true}),
            None,
        ),
        "config.patch" => (
            Value::Null,
            Some(Fault {
                code: "restart_required".into(),
                message:
                    "configuration is a startup snapshot; restart the main process to apply changes"
                        .into(),
            }),
        ),
        _ => (
            Value::Null,
            Some(Fault {
                code: "invalid_control".into(),
                message: format!("unsupported control {}", control.method),
            }),
        ),
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
pub async fn termination() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut sig) = signal(SignalKind::terminate()) {
            tokio::select! {_=sig.recv()=>{},_=console_interrupt()=>{}}
            return;
        }
    }
    console_interrupt().await;
}
async fn console_interrupt() {
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}
pub fn bounded(name: &str, value: usize, min: usize, max: usize) -> Result<()> {
    if !(min..=max).contains(&value) {
        bail!("{name} must be {min}..={max}");
    }
    Ok(())
}
pub async fn finish(client: &Client, result: &Result<()>) {
    if let Err(e) = result {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.request("report", json!({"state":"failed","error":format!("{e:#}")})),
        )
        .await;
        eprintln!("plugin failed: {e:#}");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_is_static_and_shutdown_is_not_completion() {
        let snapshot = json!({"stream":"s"});
        let control = |method: &str| Control {
            call_id: 1,
            method: method.into(),
            args: json!({"stream":"changed"}),
        };
        let (answer, error) = control_answer(&control("config.get"), &snapshot);
        assert!(error.is_none());
        assert_eq!(answer["effective"], snapshot);
        assert_eq!(
            control_answer(&control("config.patch"), &snapshot)
                .1
                .unwrap()
                .code,
            "restart_required"
        );
        assert_eq!(
            control_answer(&control("shutdown"), &snapshot).0["completed"],
            false
        );
    }
    #[tokio::test]
    async fn stop_observes_existing_and_future_signal() {
        let (tx, mut rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            stopped(&mut rx).await;
        });
        tx.send(true).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        let (_, mut rx) = watch::channel(true);
        stopped(&mut rx).await;
    }
}
