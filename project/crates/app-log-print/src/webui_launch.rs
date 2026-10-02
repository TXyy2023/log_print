use anyhow::{bail, Context, Result};
use log_proto::{Config, PluginSpec, Role};
use serde_json::json;
use std::path::{Path, PathBuf};

pub fn prepare(config: &mut Config, base: &Path, instance: &Path) -> Result<()> {
    let runtime = uuid::Uuid::new_v4().to_string();
    let path = |value: &str| -> PathBuf {
        let p = PathBuf::from(value);
        if p.is_absolute() {
            p
        } else {
            base.join(p)
        }
    };
    let mut managed = Vec::new();
    for index in 0..config.plugins.len() {
        if Path::new(&config.plugins[index].bin)
            .file_stem()
            .and_then(|v| v.to_str())
            != Some("output-webui")
        {
            continue;
        }
        let web = config.plugins[index].id.clone();
        let archive_dir = config.plugins[index].config["archive_dir"]
            .as_str()
            .map(str::to_owned);
        let history = config.plugins[index].config["history_plugin"]
            .as_str()
            .map(str::to_owned);
        if archive_dir.is_some() && history.is_some() {
            bail!("{web}: archive_dir and history_plugin are mutually exclusive");
        }
        let binding = if let Some(dir) = archive_dir {
            let id = format!("{web}-archive");
            if config.plugins.iter().any(|p| p.id == id) {
                bail!("managed archive plugin id already exists: {id}");
            }
            let dir = path(&dir);
            std::fs::create_dir_all(&dir)?;
            let archive = dir.join(format!("run-{runtime}.sqlite"));
            managed.push(PluginSpec {id:id.clone(),role:Role::Output,bin:"output-file".into(),args:vec![],autostart:true,reads:vec![],read_all:true,streams:vec![],config:json!({"streams":[],"discover_streams":true,"mode":"create","sqlite":{"path":archive},"fail_on_gap":false})});
            Some((id, archive))
        } else if let Some(id) = history {
            let archive = config
                .plugins
                .iter()
                .find(|p| p.id == id)
                .context("history_plugin not configured in this instance")?;
            if Path::new(&archive.bin).file_stem().and_then(|v| v.to_str()) != Some("output-file") {
                bail!("history_plugin must be output-file");
            }
            let file = archive
                .config
                .pointer("/sqlite/path")
                .and_then(|p| p.as_str())
                .context("history_plugin requires SQLite")?;
            Some((id, path(file)))
        } else {
            None
        };
        let spec = &mut config.plugins[index];
        spec.read_all = true;
        spec.reads.clear();
        if spec.config.get("state_path").is_none() {
            spec.config["state_path"] =
                json!(instance.join(".webui").join(&web).join("pages.sqlite3"));
        } else if let Some(p) = spec.config["state_path"].as_str() {
            spec.config["state_path"] = json!(path(p));
        }
        spec.config["runtime_id"] = json!(runtime);
        if let Some((id, file)) = binding {
            spec.config["history_path"] = json!(file);
            spec.config["history_plugin"] = json!(id);
        } else if spec.config.get("history_path").is_some() {
            bail!("use history_plugin to bind archives from this instance");
        }
    }
    config.plugins.extend(managed);
    Ok(())
}
