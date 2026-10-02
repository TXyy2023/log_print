use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use log_proto::{Config, PluginSpec, Role, StreamSpec};
use serde_json::{json, Value};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

#[derive(Parser)]
#[command(
    name = "log-print",
    version,
    about = "Collect and inspect local log streams from the terminal"
)]
pub struct Cli {
    #[arg(long, global = true, default_value = ".log-print/state.json")]
    pub state: PathBuf,
    #[command(subcommand)]
    pub command: Action,
}

#[derive(Subcommand)]
pub enum Action {
    /// Run in the foreground; Ctrl-C shuts down owned children.
    Run(Launch),
    /// Start in the background and wait for readiness.
    Start(Launch),
    Status,
    Streams,
    /// Inspect a stream using its Core-assigned UUID.
    Stream {
        id: String,
    },
    /// Change a stream's display description.
    Describe {
        stream: String,
        description: String,
    },
    /// Resolve a configuration alias to a real stream UUID.
    Resolve {
        alias: String,
    },
    /// Read the current retained snapshot as text with record metadata.
    Read {
        stream: String,
        #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(u64).range(1..=64))]
        limit: u64,
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u64).range(0..=60_000))]
        wait_ms: u64,
        /// Write only exact payload bytes; refuse pages reporting gaps.
        #[arg(long)]
        raw: bool,
    },
    Config {
        #[arg(long)]
        plugin: Option<String>,
    },
    Webui {
        id: String,
        #[command(subcommand)]
        command: Box<crate::webui_cli::WebuiAction>,
    },
    Plugin {
        #[command(subcommand)]
        command: PluginAction,
    },
    /// Call a Core operation using KEY=VALUE arguments.
    Call {
        op: String,
        #[command(flatten)]
        args: Values,
    },
    /// Stop this instance and wait for complete cleanup.
    Stop,
}

#[derive(Subcommand)]
pub enum PluginAction {
    Start {
        id: String,
        #[arg(long)]
        stream: Option<String>,
    },
    Stop {
        id: String,
    },
    Restart {
        id: String,
    },
    /// Call a plugin control method using KEY=VALUE arguments.
    Call {
        id: String,
        method: String,
        #[command(flatten)]
        args: Values,
    },
}

#[derive(Args, Default)]
pub struct Values {
    /// Named values; true/false/null/numbers are typed, other values are text.
    #[arg(value_name = "KEY=VALUE")]
    values: Vec<String>,
    #[arg(long = "arg", value_name = "KEY=VALUE")]
    named: Vec<String>,
    /// Force a value to remain text, including numbers or true/false.
    #[arg(long, value_name = "KEY=VALUE")]
    text: Vec<String>,
    /// Append a typed list item; repeat for multiple items.
    #[arg(long, value_name = "KEY=VALUE")]
    list: Vec<String>,
    /// Append a list item as text, including numeric strings.
    #[arg(long, value_name = "KEY=VALUE")]
    list_text: Vec<String>,
    /// Supply an empty list.
    #[arg(long, value_name = "KEY")]
    empty_list: Vec<String>,
}

fn assignment(value: &str) -> Result<(&str, &str)> {
    let (key, value) = value.split_once('=').context("expected KEY=VALUE")?;
    if key.is_empty() || key.split('.').any(str::is_empty) {
        bail!("argument key must contain nonempty path components")
    }
    Ok((key, value))
}

fn scalar(value: &str) -> Value {
    match value {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => serde_json::from_str::<serde_json::Number>(value)
            .map(Value::Number)
            .unwrap_or_else(|_| Value::String(value.into())),
    }
}

fn insert(object: &mut Value, key: &str, value: Value, append: bool) -> Result<()> {
    let mut target = object;
    let mut parts = key.split('.').peekable();
    while let Some(part) = parts.next() {
        let map = target
            .as_object_mut()
            .context("argument path conflicts with a scalar value")?;
        if parts.peek().is_some() {
            target = map.entry(part).or_insert_with(|| json!({}));
        } else {
            if append {
                map.entry(part)
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
                    .context("list argument conflicts with a scalar value")?
                    .push(value);
            } else if map.insert(part.into(), value).is_some() {
                bail!("duplicate argument {key}; use --list for multiple items")
            }
            return Ok(());
        }
    }
    bail!("empty argument key")
}

impl Values {
    pub fn parse(self) -> Result<Value> {
        let mut value = json!({});
        for item in self.values.iter().chain(&self.named) {
            let (key, text) = assignment(item)?;
            insert(&mut value, key, scalar(text), false)?;
        }
        for item in self.text {
            let (key, text) = assignment(&item)?;
            insert(&mut value, key, Value::String(text.into()), false)?;
        }
        for key in self.empty_list {
            if key.is_empty() || key.split('.').any(str::is_empty) {
                bail!("invalid list key")
            }
            insert(&mut value, &key, json!([]), false)?;
        }
        for (item, is_text) in self
            .list
            .into_iter()
            .map(|v| (v, false))
            .chain(self.list_text.into_iter().map(|v| (v, true)))
        {
            let (key, text) = assignment(&item)?;
            insert(
                &mut value,
                key,
                if is_text { json!(text) } else { scalar(text) },
                true,
            )?;
        }
        Ok(value)
    }
}

#[derive(Args, Default)]
pub struct Launch {
    /// Optional existing configuration file; cannot be mixed with launch flags.
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// Follow a file; use --set ID.mode=static for a one-time import.
    #[arg(long, value_name = "ID=PATH")]
    input_file: Vec<String>,
    /// Spawn a program; pass program arguments with --list ID.args=VALUE.
    #[arg(long, value_name = "ID=COMMAND")]
    input_program: Vec<String>,
    #[arg(long, value_name = "ID=PANE")]
    input_tmux: Vec<String>,
    #[arg(long, value_name = "ID")]
    output_raw: Vec<String>,
    #[arg(long, value_name = "ID")]
    output_transform: Vec<String>,
    /// Archive a single stream to a new raw file.
    #[arg(long, value_name = "ID=PATH")]
    output_file: Vec<String>,
    #[arg(long, value_name = "ID=PATH")]
    output_sqlite: Vec<String>,
    #[arg(long, value_name = "ID")]
    output_webui: Vec<String>,
    #[arg(long, value_name = "WEB=DIRECTORY")]
    webui_archive: Vec<String>,
    #[arg(long, value_name = "WEB=ARCHIVE_PLUGIN")]
    webui_history: Vec<String>,
    /// Declare a custom input plugin.
    #[arg(long, value_name = "ID=BINARY")]
    input: Vec<String>,
    /// Declare a custom output plugin.
    #[arg(long, value_name = "ID=BINARY")]
    output: Vec<String>,
    #[arg(long, value_name = "KEY=VALUE")]
    core: Vec<String>,
    /// Set a plugin configuration field; dots address nested fields.
    #[arg(long, value_name = "ID.KEY=VALUE")]
    set: Vec<String>,
    #[arg(long, value_name = "ID.KEY=VALUE")]
    text: Vec<String>,
    /// Append a typed plugin configuration list item.
    #[arg(long, value_name = "ID.KEY=VALUE")]
    list: Vec<String>,
    /// Append a plugin configuration list item as text.
    #[arg(long, value_name = "ID.KEY=VALUE")]
    list_text: Vec<String>,
    /// Select an output's source alias; repeat for multiple streams.
    #[arg(long, value_name = "ID=ALIAS")]
    read: Vec<String>,
    #[arg(long, value_name = "ID=TEXT")]
    describe: Vec<String>,
    #[arg(long, value_name = "ID")]
    no_autostart: Vec<String>,
    /// Append an argument to the plugin executable itself.
    #[arg(long, value_name = "ID=ARGUMENT")]
    plugin_arg: Vec<String>,
}

impl Launch {
    fn fields(&self) -> [(&str, &Vec<String>); 21] {
        [
            ("--input-file", &self.input_file),
            ("--input-program", &self.input_program),
            ("--input-tmux", &self.input_tmux),
            ("--output-raw", &self.output_raw),
            ("--output-transform", &self.output_transform),
            ("--output-file", &self.output_file),
            ("--output-sqlite", &self.output_sqlite),
            ("--output-webui", &self.output_webui),
            ("--webui-archive", &self.webui_archive),
            ("--webui-history", &self.webui_history),
            ("--input", &self.input),
            ("--output", &self.output),
            ("--core", &self.core),
            ("--set", &self.set),
            ("--text", &self.text),
            ("--list", &self.list),
            ("--list-text", &self.list_text),
            ("--read", &self.read),
            ("--describe", &self.describe),
            ("--no-autostart", &self.no_autostart),
            ("--plugin-arg", &self.plugin_arg),
        ]
    }

    pub fn child_args(&self) -> Result<Vec<OsString>> {
        if let Some(path) = &self.config {
            return Ok(vec![
                "--config".into(),
                std::fs::canonicalize(path)?.into_os_string(),
            ]);
        }
        Ok(self
            .fields()
            .into_iter()
            .flat_map(|(flag, values)| {
                values
                    .iter()
                    .map(move |value| format!("{flag}={value}").into())
            })
            .collect())
    }

    pub fn load(&self) -> Result<(Config, PathBuf, String)> {
        if let Some(path) = &self.config {
            if self.fields().iter().any(|(_, values)| !values.is_empty()) {
                bail!("--config cannot be mixed with command-line launch settings")
            }
            let path = std::fs::canonicalize(path).context("config file unavailable")?;
            let bytes = std::fs::read(&path)?;
            if bytes.len() > log_proto::MAX_WIRE {
                bail!("configuration exceeds 1 MiB")
            }
            let config = serde_json::from_slice(&bytes).context("invalid configuration")?;
            return Ok((
                config,
                path.parent().context("config parent")?.into(),
                path.display().to_string(),
            ));
        }
        let mut plugins = BTreeMap::<String, PluginSpec>::new();
        let mut archive_paths = BTreeMap::new();
        for (items, bin, role, field, mode) in [
            (
                &self.input_file,
                "input-file",
                Role::Input,
                "path",
                "follow",
            ),
            (
                &self.input_program,
                "input-program",
                Role::Input,
                "command",
                "spawn",
            ),
            (
                &self.input_tmux,
                "input-program",
                Role::Input,
                "tmux_target",
                "tmux",
            ),
            (&self.output_file, "output-file", Role::Output, "", "create"),
            (
                &self.output_sqlite,
                "output-file",
                Role::Output,
                "sqlite.path",
                "create",
            ),
            (&self.input, "", Role::Input, "", ""),
            (&self.output, "", Role::Output, "", ""),
        ] {
            for item in items {
                let (id, argument) = assignment(item)?;
                if argument.is_empty() {
                    bail!("{id}: path or executable must not be empty")
                }
                let bin = if bin.is_empty() { argument } else { bin };
                let mut config = json!({});
                if !mode.is_empty() {
                    config["mode"] = json!(mode);
                }
                if !field.is_empty() {
                    insert(&mut config, field, json!(argument), false)?;
                }
                if bin == "output-file" && mode == "create" && field.is_empty() {
                    config["file"] = json!({"format":"raw","paths":{}});
                    archive_paths.insert(id.to_owned(), argument.to_owned());
                }
                add_plugin(&mut plugins, id, bin, role, config)?;
            }
        }
        for (items, bin) in [
            (&self.output_webui, "output-webui"),
            (&self.output_raw, "output-raw"),
            (&self.output_transform, "output-transform"),
        ] {
            for id in items {
                add_plugin(&mut plugins, id, bin, Role::Output, json!({}))?;
            }
        }
        for plugin in plugins.values_mut() {
            if plugin.bin == "output-webui" {
                plugin.read_all = true;
            }
        }
        for (items, key) in [
            (&self.webui_archive, "archive_dir"),
            (&self.webui_history, "history_plugin"),
        ] {
            for item in items {
                let (id, value) = assignment(item)?;
                let plugin = plugins.get_mut(id).context("unknown WebUI")?;
                if plugin.bin != "output-webui" || value.is_empty() {
                    bail!("archive/history flag requires a WebUI and nonempty value");
                }
                plugin.config[key] = json!(value);
            }
        }
        for item in &self.read {
            let (id, alias) = assignment(item)?;
            let plugin = plugins
                .get_mut(id)
                .context("--read refers to an unknown plugin")?;
            if plugin.role != Role::Output || alias.is_empty() {
                bail!("--read requires an output and a nonempty stream")
            }
            plugin.reads.push(alias.into());
        }
        let sources: Vec<_> = plugins
            .values()
            .filter(|p| p.role == Role::Input)
            .map(|p| p.id.clone())
            .collect();
        for plugin in plugins.values_mut() {
            if plugin.role == Role::Output && plugin.reads.is_empty() && !plugin.read_all {
                plugin.reads = sources.clone();
            }
            if matches!(
                plugin.bin.as_str(),
                "output-raw" | "output-transform" | "output-file"
            ) {
                plugin.config["streams"] = json!(plugin.reads);
            }
            if plugin.bin == "output-transform" {
                plugin.streams.push(StreamSpec {
                    id: plugin.id.clone(),
                    description: String::new(),
                    parents: plugin.reads.clone(),
                });
                plugin.config["output_stream"] = json!(plugin.id);
            }
            if let Some(path) = archive_paths.get(&plugin.id) {
                if plugin.reads.len() != 1 {
                    bail!("--output-file {} requires exactly one source; select it with --read {}=ALIAS", plugin.id, plugin.id)
                }
                plugin.config["file"]["paths"][&plugin.reads[0]] = json!(path);
            }
        }
        let mut explicit_fields = BTreeMap::new();
        for (items, text, list) in [
            (&self.set, false, false),
            (&self.text, true, false),
            (&self.list, false, true),
            (&self.list_text, true, true),
        ] {
            for item in items {
                let (path, value) = assignment(item)?;
                let id = plugins
                    .keys()
                    .filter(|id| path.starts_with(&format!("{id}.")))
                    .max_by_key(|id| id.len())
                    .cloned()
                    .context(
                        "configuration field must start with a declared plugin ID and a dot",
                    )?;
                let key = &path[id.len() + 1..];
                let plugin = plugins.get_mut(&id).unwrap();
                // The first explicit field replaces shortcut defaults; only lists repeat.
                match explicit_fields.insert(path.to_owned(), list) {
                    Some(previous) if !list || !previous => {
                        bail!("duplicate configuration field {path}; use --list for multiple items")
                    }
                    None => remove(&mut plugin.config, key)?,
                    _ => (),
                }
                insert(
                    &mut plugin.config,
                    key,
                    if text { json!(value) } else { scalar(value) },
                    list,
                )?;
            }
        }
        for item in &self.describe {
            let (id, description) = assignment(item)?;
            let plugin = plugins
                .get_mut(id)
                .context("unknown plugin in --describe")?;
            let stream = plugin
                .streams
                .first_mut()
                .context("--describe requires a publishing plugin")?;
            stream.description = description.into();
        }
        for id in &self.no_autostart {
            plugins
                .get_mut(id)
                .context("unknown plugin in --no-autostart")?
                .autostart = false;
        }
        for item in &self.plugin_arg {
            let (id, argument) = assignment(item)?;
            plugins
                .get_mut(id)
                .context("unknown plugin in --plugin-arg")?
                .args
                .push(argument.into());
        }
        let core = Values {
            named: self.core.clone(),
            ..Values::default()
        }
        .parse()?;
        let config = Config {
            core: serde_json::from_value(core).context("invalid --core settings")?,
            plugins: plugins.into_values().collect(),
        };
        if serde_json::to_vec(&config)?.len() > log_proto::MAX_WIRE {
            bail!("configuration exceeds 1 MiB")
        }
        Ok((config, std::env::current_dir()?, "command line".into()))
    }
}

fn remove(object: &mut Value, key: &str) -> Result<()> {
    if let Some((first, rest)) = key.split_once('.') {
        if let Some(child) = object.get_mut(first) {
            remove(child, rest)?;
        }
    } else {
        object
            .as_object_mut()
            .context("configuration path conflicts with a scalar")?
            .remove(key);
    }
    Ok(())
}

fn add_plugin(
    plugins: &mut BTreeMap<String, PluginSpec>,
    id: &str,
    bin: &str,
    role: Role,
    config: Value,
) -> Result<()> {
    if id.is_empty() || id.starts_with("__") || id.len() > 100 || plugins.contains_key(id) {
        bail!("duplicate, empty or reserved plugin ID: {id}")
    }
    plugins.insert(
        id.into(),
        PluginSpec {
            read_all: false,
            id: id.into(),
            role,
            bin: bin.into(),
            args: vec![],
            autostart: true,
            reads: vec![],
            streams: if role == Role::Input {
                vec![StreamSpec {
                    id: id.into(),
                    description: String::new(),
                    parents: vec![],
                }]
            } else {
                vec![]
            },
            config,
        },
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_values_preserve_types_lists_and_nested_strings() {
        let cli = Cli::try_parse_from([
            "log-print",
            "call",
            "test",
            "count=3",
            "enabled=true",
            "absent=null",
            "--text",
            "label=001",
            "--text",
            "env.FLAG=false",
            "--list",
            "bytes=0",
            "--list",
            "bytes=255",
            "--list-text",
            "labels=001",
            "--empty-list",
            "parents",
        ])
        .unwrap();
        let Action::Call { args, .. } = cli.command else {
            panic!()
        };
        assert_eq!(
            args.parse().unwrap(),
            json!({"count":3,"enabled":true,"absent":null,
            "label":"001","env":{"FLAG":"false"},"bytes":[0,255],"labels":["001"],"parents":[]})
        );
        for args in [vec!["x=1", "x=2"], vec!["x=1", "x.y=2"]] {
            assert!(Values {
                values: args.into_iter().map(String::from).collect(),
                ..Values::default()
            }
            .parse()
            .is_err());
        }
    }

    #[test]
    fn launch_links_sources_archives_and_derived_streams() {
        let cli = Cli::try_parse_from([
            "log-print",
            "start",
            "--input-file",
            "source=路径/a=b.log",
            "--set",
            "source.from_start=true",
            "--output-transform",
            "derived",
            "--set",
            "derived.number=true",
            "--output-file",
            "archive=capture.log",
            "--read",
            "archive=derived",
            "--no-autostart",
            "archive",
            "--describe",
            "source=编译日志",
            "--core",
            "transport=udp",
        ])
        .unwrap();
        let Action::Start(launch) = cli.command else {
            panic!()
        };
        let (config, _, source) = launch.load().unwrap();
        assert_eq!(source, "command line");
        let file = config.plugins.iter().find(|p| p.id == "source").unwrap();
        assert_eq!(file.config["path"], "路径/a=b.log");
        assert_eq!(file.config["from_start"], true);
        assert_eq!(file.streams[0].description, "编译日志");
        let archive = config.plugins.iter().find(|p| p.id == "archive").unwrap();
        assert_eq!(archive.reads, ["derived"]);
        assert_eq!(archive.config["file"]["paths"]["derived"], "capture.log");
        assert!(!archive.autostart);
        let derived = config.plugins.iter().find(|p| p.id == "derived").unwrap();
        assert_eq!(derived.streams[0].parents, ["source"]);
        let mut child = vec![OsString::from("log-print"), OsString::from("run")];
        child.extend(launch.child_args().unwrap());
        let Action::Run(roundtrip) = Cli::try_parse_from(child).unwrap().command else {
            panic!()
        };
        assert_eq!(
            serde_json::to_value(config).unwrap(),
            serde_json::to_value(roundtrip.load().unwrap().0).unwrap()
        );
    }

    #[test]
    fn rejects_ambiguous_launch_and_removed_json_interface() {
        for args in [
            vec!["--input-file", "source=a", "--input-file", "source=b"],
            vec!["--input-file", "source=a", "--set", "missing.mode=static"],
            vec![
                "--input-file",
                "source=a",
                "--input-file",
                "other=b",
                "--output-file",
                "archive=c",
            ],
            vec!["--core", "unknown=1"],
            vec!["--config", "unused.json", "--output-raw", "screen"],
        ] {
            let Action::Start(launch) =
                Cli::try_parse_from([vec!["log-print", "start"], args].concat())
                    .unwrap()
                    .command
            else {
                panic!()
            };
            assert!(launch.load().is_err());
        }
        assert!(Cli::try_parse_from(["log-print", "call", "streams", "--json", "{}"]).is_err());
        assert!(Cli::try_parse_from(["log-print", "read", "uuid", "--limit", "0"]).is_err());
    }
}
