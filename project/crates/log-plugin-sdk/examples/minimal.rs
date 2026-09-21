use std::collections::BTreeMap;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _events, _controls) = log_plugin_sdk::connect_env().await?;
    let stream = client
        .stream_id()
        .ok_or_else(|| anyhow::anyhow!("declare one input stream"))?;
    let outcome = client
        .publish_tagged(
            stream,
            "example:1",
            b"hello\0world".to_vec(),
            None,
            BTreeMap::new(),
            None,
            Some(1),
        )
        .await?;
    eprintln!("{outcome:?}");
    Ok(())
}
