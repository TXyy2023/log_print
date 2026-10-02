use anyhow::{bail, Context, Result};
use futures_util::{stream, StreamExt};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use std::{collections::BTreeMap, time::Duration};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Default)]
pub struct Snapshot {
    pub state: Value,
    pub panels: BTreeMap<String, Value>,
    pub connected: bool,
    pub notice: String,
    pub reply: String,
}
#[derive(Clone)]
pub enum After {
    SelectPage,
    SelectPanel,
    ShowQuery { page: String, panel: String },
}
pub struct Call {
    pub method: String,
    pub args: Value,
    pub after: Option<After>,
}
impl Call {
    pub fn new(method: &str, args: Value) -> Self {
        Self {
            method: method.into(),
            args,
            after: None,
        }
    }
}
#[derive(Clone)]
pub struct Connection {
    client: Client,
    url: Url,
}
impl Connection {
    pub fn new(address: &str) -> Result<Self> {
        let url = Url::parse(address)?;
        let local = url.host_str().is_some_and(|h| {
            h.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        if url.scheme() != "http"
            || !local
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            bail!("attach requires a loopback HTTP display URL reported by log-print");
        }
        Ok(Self {
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(4))
                .build()?,
            url,
        })
    }
    async fn json(&self, path: &str, call: Option<(&str, Value)>) -> Result<Value> {
        let url = self.url.join(path)?;
        let request = if let Some((method, args)) = call {
            self.client
                .post(url)
                .header("Origin", self.url.as_str().trim_end_matches('/'))
                .json(&json!({"method":method,"args":args}))
        } else {
            self.client.get(url)
        };
        let mut response = request.send().await?;
        let status = response.status();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 4 * 1024 * 1024 {
                bail!("display response exceeds 4 MiB");
            }
            bytes.extend_from_slice(&chunk);
        }
        let result: Value = serde_json::from_slice(&bytes).context("invalid display response")?;
        if !status.is_success() {
            bail!(
                "{}",
                result["error"].as_str().unwrap_or("display request failed")
            );
        }
        Ok(result)
    }
    pub async fn command(&self, method: &str, args: Value) -> Result<Value> {
        self.json("api/control", Some((method, args))).await
    }
    pub async fn state(&self) -> Result<Value> {
        self.json("api/state", None).await
    }
    async fn panel(&self, page: &str, panel: &Value) -> Result<Value> {
        if panel["paused"] != true && panel["mode"] == "history" {
            let query = panel["query"]
                .as_str()
                .context("history query expired; start history again")?;
            let mut data = self
                .command("query.get", json!({"query":query,"offset":panel["offset"]}))
                .await?;
            if panel["kind"] == "curve" && data["status"]["state"] == "complete" {
                while data["next"].as_u64().unwrap_or(0) < data["total"].as_u64().unwrap_or(0)
                    && data["rows"].as_array().map_or(0, Vec::len) < 2000
                {
                    let batch = self
                        .command("query.get", json!({"query":query,"offset":data["next"]}))
                        .await?;
                    if batch["next"].as_u64().unwrap_or(0) <= data["next"].as_u64().unwrap_or(0) {
                        break;
                    }
                    data["rows"]
                        .as_array_mut()
                        .context("rows")?
                        .extend(batch["rows"].as_array().context("rows")?.iter().cloned());
                    data["next"] = batch["next"].clone();
                }
            }
            Ok(data)
        } else {
            self.command("panel.data", json!({"page":page,"panel":panel["id"]}))
                .await
        }
    }
    pub async fn refresh(&self, snapshot: &mut Snapshot) -> Result<()> {
        let state = self.state().await?;
        let page = state["pages"]
            .as_array()
            .context("not a display service")?
            .iter()
            .find(|p| p["id"] == state["selected"])
            .context("selected page")?;
        let page_id = page["id"].as_str().context("page id")?;
        let visible: Vec<_> = page["panels"]
            .as_array()
            .context("panels")?
            .iter()
            .filter(|p| p["hidden"] != true)
            .cloned()
            .collect();
        let mut pending = stream::iter(visible.into_iter().map(|p| async move {
            let result = self
                .panel(page_id, &p)
                .await
                .unwrap_or_else(|e| json!({"error":format!("{e:#}")}));
            (p["id"].as_str().unwrap_or_default().to_owned(), result)
        }))
        .buffer_unordered(4);
        let mut panels = BTreeMap::new();
        let mut used = 0;
        while let Some((id, mut data)) = pending.next().await {
            used += serde_json::to_vec(&data)?.len();
            if used > 64 * 1024 * 1024 {
                data = json!({"error":"terminal display cache budget reached (64 MiB); hide some panels"});
            }
            panels.insert(id, data);
        }
        drop(pending);
        snapshot.state = state;
        snapshot.panels = panels;
        snapshot.connected = true;
        Ok(())
    }
    async fn execute(&self, call: Call) -> Result<Value> {
        let result = self.command(&call.method, call.args.clone()).await?;
        match call.after {
            Some(After::SelectPage) => { self.command("page.select",json!({"page":result["result"]["id"],"revision":result["revision"]})).await?; }
            Some(After::SelectPanel) => { self.command("page.set",json!({"page":call.args["page"],"active_panel":result["result"]["id"],"revision":result["revision"]})).await?; }
            Some(After::ShowQuery{page,panel}) => {
                if let Err(error)=self.command("panel.set",json!({"page":page,"panel":panel,"query":result["query"],"mode":"history","offset":0,"paused":false,"revision":call.args["revision"]})).await {
                    let _=self.command("query.cancel",json!({"query":result["query"]})).await;
                    return Err(error);
                }
            }
            None=>{}
        }
        Ok(result)
    }
}
pub fn worker(
    connection: Connection,
) -> (
    mpsc::Sender<Call>,
    watch::Receiver<Snapshot>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = mpsc::channel::<Call>(32);
    let (updates, view) = watch::channel(Snapshot::default());
    let task = tokio::spawn(async move {
        let mut snapshot = Snapshot::default();
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                call=rx.recv()=>match call {
                    Some(call)=>{
                        let method=call.method.clone();
                        match connection.execute(call).await {
                            Ok(result)=>{ snapshot.notice=format!("Saved · {method}"); snapshot.reply=serde_json::to_string_pretty(&result).unwrap_or_default().chars().take(65536).collect(); },
                            Err(error)=>{snapshot.notice=format!("Not saved · {error:#}");snapshot.reply=snapshot.notice.clone();}
                        }
                    },
                    None=>break,
                },
                _=tick.tick()=>{}
            }
            if let Err(error) = connection.refresh(&mut snapshot).await {
                snapshot.connected = false;
                snapshot.notice = format!("Disconnected · {error:#}");
            }
            updates.send_replace(snapshot.clone());
        }
    });
    (tx, view, task)
}
