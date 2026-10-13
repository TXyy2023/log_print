use crate::engine::Engine;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{path::PathBuf, sync::Arc, time::Duration};

impl Engine {
    pub async fn import_file(self: &Arc<Self>, name: String, payload: &[u8]) -> Result<Value> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = self.working_dir("imports")?.join(format!("{id}.log"));
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await?;
        use tokio::io::AsyncWriteExt;
        file.write_all(payload).await?;
        file.sync_all().await?;
        drop(file);
        let prepared = self
            .manager(
                "webui.import.prepare",
                json!({"web":std::env::var("LOG_PRINT_PLUGIN")?,"path":path,"name":name}),
            )
            .await;
        let prepared = match prepared {
            Ok(v) => v,
            Err(e) => {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(e);
            }
        };
        let stream = prepared["stream"]["id"]
            .as_str()
            .context("stream id")?
            .to_owned();
        let recording = self
            .recording_start(json!({"stream":stream,"mode":"now"}))
            .await?;
        let size = payload.len() as u64;
        let status = json!({"id":id,"name":name,"stream":stream,"state":"importing","bytes":size.to_string(),"archive":recording["archive"],"recording":recording["id"]});
        self.imports
            .lock()
            .unwrap()
            .insert(id.clone(), status.clone());
        let engine = self.clone();
        tokio::spawn(async move {
            let result = engine.run_import(&prepared, &recording, &path, size).await;
            let mut imports = engine.imports.lock().unwrap();
            let status = imports.get_mut(&id).unwrap();
            match result {
                Ok(()) => {
                    status["state"] = json!("complete");
                }
                Err(e) => {
                    status["state"] = json!("failed");
                    status["error"] = json!(format!("{e:#}"));
                }
            }
            drop(imports);
            let _ = engine.updates.send(());
        });
        Ok(status)
    }
    async fn run_import(
        &self,
        prepared: &Value,
        recording: &Value,
        path: &PathBuf,
        size: u64,
    ) -> Result<()> {
        let id = recording["id"].as_str().context("recording")?;
        let result: Result<()> = async {
            // Do not release the producer before output-file has its initial cursor.
            loop {
                let state = self.recordings.lock().unwrap()[id]
                    .status
                    .lock()
                    .unwrap()
                    .clone();
                if state["state"] == "failed" {
                    bail!("import output failed: {}", state["error"]);
                }
                if state["state"] == "recording" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            self.manager("webui.import.start", json!({"id":prepared["plugin"],"checkpoint":format!("{}.checkpoint.json",recording["path"].as_str().context("recording path")?)}))
                .await?;
            loop {
                let output = self.recordings.lock().unwrap()[id].status.lock().unwrap().clone();
                if output["state"] == "failed" { bail!("import output failed: {}", output["error"]); }
                let status = self
                    .client
                    .request("plugin.status", json!({"plugin":prepared["plugin"]}))
                    .await?;
                if status["report"]["state"] == "source_eof" {
                    break;
                }
                if status["connected"] == false || status["report"]["state"] == "failed" {
                    bail!("input-file failed: {status}");
                }
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            Ok(())
        }
        .await;
        let stopped = self.recording_stop(id).await?;
        if result.is_err() {
            let _ = self
                .manager("plugin.stop", json!({"id":prepared["plugin"]}))
                .await;
        }
        result?;
        if stopped["state"] != "stopped"
            || stopped["bytes"]
                .as_str()
                .and_then(|n| n.parse::<u64>().ok())
                != Some(size)
            || stopped["writer"]["sqlite"]["incomplete"] == true
        {
            bail!("import incomplete: output-file did not confirm all {size} bytes (state {}, confirmed {}, error {}); uploaded source retained at {}",stopped["state"],stopped["bytes"],stopped["error"],path.display());
        }
        tokio::fs::remove_file(path).await?;
        Ok(())
    }
}
