mod transform;
use anyhow::{bail, Result};
use io_plugin_util::stopped;
use log_plugin_sdk::Event;
use log_proto::Record;
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use transform::{Config, Processor};
fn config(v: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(v.clone())?;
    c.validate()?;
    Ok(c)
}
fn validate(v: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(v)?)?)
}
struct Parent {
    processor: Processor,
    last: Record,
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut cx =
        io_plugin_util::connect(validate, &["prefix", "suffix", "delete", "replace"]).await?;
    let client = cx.client.clone();
    let r = run(&mut cx).await;
    io_plugin_util::finish(&client, &r).await;
    r
}
async fn emit(
    client: &log_plugin_sdk::Client,
    c: &Config,
    parent: &Record,
    out: transform::Output,
    index: &mut u64,
    run: uuid::Uuid,
    upstream: &BTreeMap<String, u64>,
) -> Result<()> {
    for warning in out.warnings {
        eprintln!("{warning}");
        client.request("report",json!({"state":"transform_warning","stream":parent.stream,"seq":parent.seq,"warning":warning})).await?;
    }
    for bytes in out.bytes {
        for chunk in bytes.chunks(log_proto::MAX_PAYLOAD) {
            let key = format!("{run}:{}:{}:{index}", parent.stream, parent.seq);
            client
                .publish_retained(
                    &c.output_stream,
                    &key,
                    chunk.to_vec(),
                    parent.source_ts_ns,
                    upstream.clone(),
                )
                .await?;
            *index += 1;
        }
    }
    Ok(())
}
async fn run(cx: &mut io_plugin_util::Context) -> Result<()> {
    let c = config(&cx.config.borrow())?;
    for stream in &c.streams {
        cx.client.subscribe(stream, c.from).await?;
    }
    let run = uuid::Uuid::new_v4();
    let mut parents: BTreeMap<String, Parent> = BTreeMap::new();
    let mut index = 0;
    let mut upstream = BTreeMap::new();
    let mut pending = VecDeque::new();
    let mut pending_bytes = 0usize;
    cx.client.request("report",json!({"state":"transforming","streams":c.streams,"output_stream":c.output_stream,"run":run,"original_bytes":"unchanged in parent stream"})).await?;
    loop {
        let event = tokio::select! {_=stopped(&mut cx.shutdown)=>break,event=cx.events.recv()=>event.ok_or_else(||anyhow::anyhow!("event connection closed"))?};
        let c = config(&cx.config.borrow())?;
        match event {
            Event::Record(r) => {
                let state = parents.entry(r.stream.clone()).or_insert_with(|| Parent {
                    processor: Processor::new(&c),
                    last: r.clone(),
                });
                if state.last.epoch != r.epoch {
                    bail!("parent epoch changed; manual restart required to avoid joining distinct runs")
                }
                let out = state.processor.feed(&r.payload, false, &c)?;
                state.last = r;
                upstream.insert(state.last.stream.clone(), state.last.seq);
                pending_bytes += out.bytes.iter().map(Vec::len).sum::<usize>();
                if !out.bytes.is_empty() || !out.warnings.is_empty() {
                    pending.push_back((state.last.clone(), out));
                }
                if upstream.len() < c.streams.len() {
                    if pending_bytes > c.max_pending_bytes || pending.len() > 64 {
                        bail!("waiting for all parents exceeded bounded pending output; missing parent has not published")
                    }
                } else {
                    while let Some((parent, out)) = pending.pop_front() {
                        tokio::select! {_=stopped(&mut cx.shutdown)=>{eprintln!("shutdown interrupted derived publish; current parent outcome may be unknown");return Ok(())},r=emit(&cx.client,&c,&parent,out,&mut index,run,&upstream)=>r?}
                    }
                    pending_bytes = 0;
                }
            }
            Event::Disconnected { stream, reason } => {
                bail!("parent event connection lost for {stream}: {reason}")
            }
            Event::Gap {
                stream,
                epoch,
                from,
                to,
                reason,
            } => {
                cx.client.request("report",json!({"state":"parent_gap","stream":stream,"epoch":epoch,"from":from,"to":to,"reason":reason})).await?;
                bail!("parent gap; stopped rather than joining unrelated partial records")
            }
        }
    }
    let c = config(&cx.config.borrow())?;
    if !pending.is_empty() {
        bail!("shutdown before all parents arrived; pending derived output not published")
    }
    for state in parents.values_mut() {
        let pending = state.processor.pending();
        let out = state.processor.feed(b"", true, &c)?;
        match tokio::time::timeout(std::time::Duration::from_secs(2),emit(&cx.client,&c,&state.last,out,&mut index,run,&upstream)).await{Ok(r)=>r?,Err(_)=>bail!("shutdown flush timed out; at least {pending} buffered bytes have unconfirmed derived publication")}
    }
    Ok(())
}
