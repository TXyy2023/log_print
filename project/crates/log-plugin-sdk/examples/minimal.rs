use std::collections::BTreeMap;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (client, _events, _controls) = log_plugin_sdk::connect_env().await?;
    let stream = client.config()["stream"].as_str().unwrap_or("example");
    let key = format!("{}:1", uuid::Uuid::new_v4());
    let record = client
        .publish_retained(
            stream,
            &key,
            b"hello\0world".to_vec(),
            None,
            BTreeMap::new(),
        )
        .await?;
    client
        .request(
            "report",
            serde_json::json!({"state":"completed","seq":record.seq}),
        )
        .await?;
    Ok(())
}
