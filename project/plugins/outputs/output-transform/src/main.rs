mod transform;
use anyhow::{bail, Result};
use log_plugin_sdk::{stopped, Event};
use log_proto::Record;
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Instant};
use transform::{Config, Processor};
fn validate(v: &Value) -> Result<Value> {
    let c: Config = serde_json::from_value(v.clone())?;
    c.validate()?;
    Ok(serde_json::to_value(c)?)
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut cx = log_plugin_sdk::connect(validate).await?;
    let result = run(&mut cx).await.and_then(|_| cx.shutdown_result());
    log_plugin_sdk::finish(&cx.client, &result).await;
    result
}
async fn emit(
    client: &log_plugin_sdk::Client,
    output: &str,
    processor: &mut Processor,
    records: Vec<Record>,
    number: &mut u64,
    run: &str,
) -> Result<()> {
    for record in records {
        let bytes = processor.decorate(&record)?;
        *number = number
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("derived source sequence exhausted"))?;
        let upstream = BTreeMap::from([(record.stream.clone(), record.seq)]);
        let source_seq = processor.next_source_sequence(record.channel.clone())?;
        client
            .publish_tagged(
                output,
                &format!("{run}:{number}"),
                bytes,
                record.source_ts_ns,
                upstream,
                record.channel,
                Some(source_seq),
            )
            .await?;
    }
    Ok(())
}
async fn run(cx: &mut log_plugin_sdk::Context) -> Result<()> {
    let mut c: Config = serde_json::from_value(cx.config.clone())?;
    if !cx.client.read_streams().is_empty() {
        c.streams = cx.client.read_streams().to_vec();
    }
    if c.streams.is_empty() {
        bail!("configure or attach a source stream");
    }
    let output = if let Some(id) = cx.client.stream_id() {
        id.to_owned()
    } else {
        cx.client.resolve_stream(&c.output_stream).await?
    };
    for stream in &mut c.streams {
        *stream = cx.client.resolve_stream(stream).await?;
        if *stream == output {
            bail!("derived output must differ from every original stream");
        }
    }
    for stream in &c.streams {
        cx.client.subscribe(stream).await?;
    }
    let mut processor = Processor::new(&c);
    let mut number = 0;
    let run = uuid::Uuid::new_v4().to_string();
    cx.client.request("report",json!({"state":"transforming","streams":c.streams,"output_stream":output,"number":c.number,"timestamp":c.timestamp,"reorder":c.reorder})).await?;
    loop {
        let deadline = processor.next_deadline();
        let expiry = async {
            match deadline {
                Some(deadline) => tokio::time::sleep_until(deadline.into()).await,
                None => std::future::pending::<()>().await,
            }
        };
        let records = tokio::select! {
            biased;
            _=stopped(&mut cx.shutdown)=>break,
            _=expiry=>processor.expire(Instant::now()),
            event=cx.events.recv()=>match event {
                Some(Event::Record(record))=>processor.feed(record,Instant::now())?,
                Some(Event::Disconnected{stream,reason})=>bail!("source disconnected for {stream}: {reason}"),
                Some(Event::Gap{stream,from,to,..})=>{eprintln!("source missing {stream} {from}..={to}");Vec::new()},
                None=>bail!("source event channel closed"),
            },
        };
        emit(
            &cx.client,
            &output,
            &mut processor,
            records,
            &mut number,
            &run,
        )
        .await?;
    }
    // Stop acceptance first, then drain accepted SDK events and reorder state.
    cx.events.close();
    for stream in &c.streams {
        cx.client.unsubscribe(stream).await?;
    }
    while let Some(event) = cx.events.recv().await {
        if let Event::Record(record) = event {
            let records = processor.feed(record, Instant::now())?;
            emit(
                &cx.client,
                &output,
                &mut processor,
                records,
                &mut number,
                &run,
            )
            .await?;
        }
    }
    let records = processor.flush();
    emit(
        &cx.client,
        &output,
        &mut processor,
        records,
        &mut number,
        &run,
    )
    .await?;
    cx.client.request("report",json!({"state":"stopped","published":number,"duplicates":processor.stats.duplicates,"skipped_source_sequences":processor.stats.skipped,"records_without_source_sequence":processor.stats.missing_source_seq,"pending":processor.pending().0})).await?;
    Ok(())
}
