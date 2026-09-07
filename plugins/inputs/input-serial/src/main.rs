use anyhow::{bail, Result};
use io_plugin_util::{bounded, stopped};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio::io::AsyncReadExt;
use tokio_serial::{DataBits, FlowControl, Parity, SerialPortBuilderExt, StopBits};
#[derive(Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct Config {
    port: String,
    stream: String,
    baud: u32,
    data_bits: u8,
    stop_bits: u8,
    parity: String,
    chunk_bytes: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            port: String::new(),
            stream: "serial".into(),
            baud: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".into(),
            chunk_bytes: 4096,
        }
    }
}
fn config(v: &Value) -> Result<Config> {
    let c: Config = serde_json::from_value(v.clone())?;
    if c.port.is_empty() || c.stream.is_empty() {
        bail!("port and stream required")
    };
    bounded("baud", c.baud as usize, 50, 4000000)?;
    bounded("chunk_bytes", c.chunk_bytes, 1, log_proto::MAX_PAYLOAD)?;
    if !(5..=8).contains(&c.data_bits)
        || !(1..=2).contains(&c.stop_bits)
        || !["none", "odd", "even"].contains(&c.parity.as_str())
    {
        bail!("invalid data_bits/stop_bits/parity")
    };
    Ok(c)
}
fn validate(v: &Value) -> Result<Value> {
    Ok(serde_json::to_value(config(v)?)?)
}
#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--list") {
        let ports = tokio_serial::available_ports()?;
        println!("{}",serde_json::to_string_pretty(&ports.iter().map(|p|json!({"port":p.port_name,"type":format!("{:?}",p.port_type),"verified":false})).collect::<Vec<_>>())?);
        return Ok(());
    }
    let mut cx = io_plugin_util::connect(validate, &[]).await?;
    let client = cx.client.clone();
    let r = run(&mut cx).await;
    io_plugin_util::finish(&client, &r).await;
    r
}
async fn run(cx: &mut io_plugin_util::Context) -> Result<()> {
    let c = config(&cx.config.borrow())?;
    let bits = match c.data_bits {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    };
    let parity = match c.parity.as_str() {
        "odd" => Parity::Odd,
        "even" => Parity::Even,
        _ => Parity::None,
    };
    let mut port = tokio_serial::new(&c.port, c.baud)
        .data_bits(bits)
        .parity(parity)
        .stop_bits(if c.stop_bits == 1 {
            StopBits::One
        } else {
            StopBits::Two
        })
        .flow_control(FlowControl::None)
        .open_native_async()?;
    cx.client.request("report",json!({"state":"reading","port":c.port,"baud":c.baud,"tx":false,"flow_control":"none","physical_overflow_detection":"unavailable","lost_bytes":"unknown","note":"OS/device buffers may overflow under backpressure; no lossless hardware claim"})).await?;
    let run = uuid::Uuid::new_v4();
    let mut offset = 0u64;
    let mut buf = vec![0; c.chunk_bytes];
    loop {
        let n =
            tokio::select! {_=stopped(&mut cx.shutdown)=>return Ok(()),r=port.read(&mut buf)=>r?};
        if n == 0 {
            bail!("serial disconnected or EOF; missing byte count unknown")
        }
        let key = format!("{run}:{offset}");
        let started = std::time::Instant::now();
        tokio::select! {_=stopped(&mut cx.shutdown)=>{eprintln!("shutdown: {n} serial bytes pending acknowledgement; physical loss unknown");return Ok(())},r=cx.client.publish_retained(&c.stream,&key,buf[..n].to_vec(),None,BTreeMap::new())=>{r?;}}
        if started.elapsed().as_millis() > 100 {
            cx.client.request("report",json!({"state":"backpressure_observed","publish_wait_ms":started.elapsed().as_millis(),"possible_gap":true,"lost_bytes":"unknown","physical_overflow_detection":"unavailable","tx":false})).await?;
        }
        offset += n as u64;
    }
}
