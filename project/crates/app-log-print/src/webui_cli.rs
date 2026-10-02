use anyhow::Result;
use clap::{Args, Subcommand};
use serde_json::{json, Value};

#[derive(Subcommand)]
pub enum WebuiAction {
    Url,
    Streams(Options),
    Capabilities,
    Page {
        #[command(subcommand)]
        command: PageAction,
    },
    Panel {
        #[command(subcommand)]
        command: PanelAction,
    },
    Series {
        #[command(subcommand)]
        command: SeriesAction,
    },
    History {
        #[command(subcommand)]
        command: HistoryAction,
    },
    Query {
        #[command(subcommand)]
        command: QueryAction,
    },
}
#[derive(Subcommand)]
pub enum PageAction {
    List(Options),
    Get(Options),
    Create(Options),
    Set(Options),
    Clone(Options),
    Delete(Options),
    Select(Options),
}
#[derive(Subcommand)]
pub enum PanelAction {
    Add(Options),
    Get(Options),
    Set(Options),
    Remove(Options),
}
#[derive(Subcommand)]
pub enum SeriesAction {
    Add(Options),
    Set(Options),
    Remove(Options),
}
#[derive(Subcommand)]
pub enum HistoryAction {
    Read(Options),
    Search(Options),
    Context(Options),
    Curve(Options),
}
#[derive(Subcommand)]
pub enum QueryAction {
    Get(Options),
    Cancel(Options),
}
/// Shared named display/query parameters. --arg supports additional dotted fields.
#[derive(Args, Default)]
pub struct Options {
    #[arg(long)]
    page: Option<String>,
    #[arg(long)]
    panel: Option<String>,
    #[arg(long)]
    series: Option<String>,
    #[arg(long)]
    query: Option<String>,
    #[arg(long)]
    name: Option<String>,
    #[arg(long)]
    title: Option<String>,
    #[arg(long)]
    theme: Option<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    revision: Option<u64>,
    #[arg(long)]
    order: Option<i64>,
    #[arg(long)]
    x: Option<u32>,
    #[arg(long)]
    y: Option<u32>,
    #[arg(long)]
    w: Option<u32>,
    #[arg(long)]
    h: Option<u32>,
    #[arg(long = "stream")]
    streams: Vec<String>,
    #[arg(long = "channel")]
    channels: Vec<String>,
    #[arg(long)]
    text: Option<String>,
    #[arg(long)]
    regex: Option<String>,
    #[arg(long)]
    field: Option<String>,
    #[arg(long)]
    color: Option<String>,
    #[arg(long)]
    width: Option<f64>,
    #[arg(long)]
    format: Option<String>,
    #[arg(long)]
    paused: Option<bool>,
    #[arg(long)]
    follow: Option<bool>,
    #[arg(long)]
    legend: Option<bool>,
    #[arg(long)]
    metadata: Option<bool>,
    #[arg(long)]
    mode: Option<String>,
    #[arg(long)]
    from: Option<u64>,
    #[arg(long)]
    end: Option<u64>,
    #[arg(long)]
    seq: Option<u64>,
    #[arg(long)]
    byte_offset: Option<usize>,
    #[arg(long)]
    epoch: Option<String>,
    #[arg(long)]
    offset: Option<usize>,
    #[arg(long)]
    limit: Option<usize>,
    #[arg(long)]
    before: Option<usize>,
    #[arg(long)]
    after: Option<usize>,
    #[arg(long)]
    time_from: Option<u64>,
    #[arg(long)]
    time_end: Option<u64>,
    #[arg(long)]
    y_min: Option<f64>,
    #[arg(long)]
    y_max: Option<f64>,
    #[arg(long)]
    zoom_start: Option<f64>,
    #[arg(long)]
    zoom_end: Option<f64>,
    #[arg(long = "column")]
    columns: Vec<String>,
    #[arg(long)]
    clear_streams: bool,
    #[arg(long)]
    clear_channels: bool,
    #[arg(long)]
    clear_columns: bool,
    #[arg(long = "column-width", value_name = "COLUMN=PIXELS")]
    column_width: Vec<String>,
    #[arg(long)]
    sort_column: Option<String>,
    #[arg(long)]
    sort_order: Option<String>,
    #[arg(long = "legend-item", value_name = "NAME=true|false")]
    legend_item: Vec<String>,
    #[arg(long = "arg", value_name = "KEY=VALUE")]
    args: Vec<String>,
}
impl Options {
    fn value(self) -> Result<Value> {
        let mut v = json!({});
        macro_rules! fields {($($field:ident),*)=>{$(if let Some(value)=self.$field {v[stringify!($field)]=json!(value);})*};}
        fields!(
            page,
            panel,
            series,
            query,
            name,
            title,
            theme,
            kind,
            revision,
            order,
            x,
            y,
            w,
            h,
            text,
            regex,
            field,
            color,
            width,
            format,
            paused,
            follow,
            legend,
            metadata,
            mode,
            from,
            end,
            seq,
            byte_offset,
            epoch,
            offset,
            limit,
            before,
            after,
            time_from,
            time_end,
            y_min,
            y_max,
            zoom_start,
            zoom_end
        );
        if !self.streams.is_empty() || self.clear_streams {
            v["streams"] = json!(self.streams);
        }
        if !self.channels.is_empty() || self.clear_channels {
            v["channels"] = json!(self.channels);
        }
        if !self.columns.is_empty() || self.clear_columns {
            v["columns"] = json!(self.columns);
        }
        for item in self.column_width {
            let (column, width) = item
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--column-width requires COLUMN=PIXELS"))?;
            if v.get("column_width").is_none() {
                v["column_width"] = json!({});
            }
            v["column_width"][column] = json!(width.parse::<u32>()?);
        }
        if let Some(column) = self.sort_column {
            v["sort_column"] = json!(column);
        }
        if let Some(order) = self.sort_order {
            v["sort_order"] = json!(order);
        }
        for item in self.legend_item {
            let (name, selected) = item
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--legend-item requires NAME=true|false"))?;
            if v.get("legend_selected").is_none() {
                v["legend_selected"] = json!({});
            }
            v["legend_selected"][name] = json!(selected.parse::<bool>()?);
        }
        for item in self.args {
            let (key, value) = item
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--arg requires KEY=VALUE"))?;
            let parts: Vec<_> = key.split('.').collect();
            let mut target = &mut v;
            for key in &parts[..parts.len() - 1] {
                if target.get(*key).is_none() {
                    target[*key] = json!({});
                }
                target = &mut target[*key];
            }
            target[parts[parts.len() - 1]] =
                serde_json::from_str(value).unwrap_or_else(|_| json!(value));
        }
        Ok(v)
    }
}
impl WebuiAction {
    pub fn request(self) -> Result<(String, Value)> {
        let (method, args) = match self {
            Self::Url => ("url", None),
            Self::Streams(v) => ("streams", Some(v)),
            Self::Capabilities => ("capabilities", None),
            Self::Page { command } => match command {
                PageAction::List(v) => ("page.list", Some(v)),
                PageAction::Get(v) => ("page.get", Some(v)),
                PageAction::Create(v) => ("page.create", Some(v)),
                PageAction::Set(v) => ("page.set", Some(v)),
                PageAction::Clone(v) => ("page.clone", Some(v)),
                PageAction::Delete(v) => ("page.delete", Some(v)),
                PageAction::Select(v) => ("page.select", Some(v)),
            },
            Self::Panel { command } => match command {
                PanelAction::Add(v) => ("panel.add", Some(v)),
                PanelAction::Get(v) => ("panel.get", Some(v)),
                PanelAction::Set(v) => ("panel.set", Some(v)),
                PanelAction::Remove(v) => ("panel.remove", Some(v)),
            },
            Self::Series { command } => match command {
                SeriesAction::Add(v) => ("series.add", Some(v)),
                SeriesAction::Set(v) => ("series.set", Some(v)),
                SeriesAction::Remove(v) => ("series.remove", Some(v)),
            },
            Self::History { command } => match command {
                HistoryAction::Read(v) => ("history.read", Some(v)),
                HistoryAction::Search(v) => ("history.search", Some(v)),
                HistoryAction::Context(v) => ("history.context", Some(v)),
                HistoryAction::Curve(v) => ("history.curve", Some(v)),
            },
            Self::Query { command } => match command {
                QueryAction::Get(v) => ("query.get", Some(v)),
                QueryAction::Cancel(v) => ("query.cancel", Some(v)),
            },
        };
        Ok((
            method.into(),
            args.map(Options::value)
                .transpose()?
                .unwrap_or_else(|| json!({})),
        ))
    }
}
