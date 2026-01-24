use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use toml::Value;

use frok_common::paths::data_path;
use frok_protocol::IngressMode;

use crate::host::normalize_route_name;
use crate::route_spec::RouteSpec;

const AGENT_SECTION: &str = "agent";
const ROUTES_SECTION: &str = "routes";

pub struct ClientStore {
    path: PathBuf,
    routes: HashMap<String, RouteSpec>,
}

impl ClientStore {
    pub fn load() -> Result<Self> {
        let path = store_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create store dir {}", parent.display()))?;
        }

        let mut store = Self {
            path,
            routes: HashMap::new(),
        };

        let doc = load_doc(&store.path)?;
        store.routes = read_routes(&doc)?;

        Ok(store)
    }

    pub fn empty() -> Self {
        Self {
            path: store_path(),
            routes: HashMap::new(),
        }
    }

    pub fn routes(&self) -> Vec<(String, RouteSpec)> {
        self.routes
            .iter()
            .map(|(host, route)| (host.clone(), route.clone()))
            .collect()
    }

    pub fn upsert(&mut self, host: String, route: RouteSpec) -> Result<()> {
        let name = normalize_route_name(&host).into_owned();
        if name.is_empty() {
            anyhow::bail!("route name is empty");
        }
        self.routes.insert(name, route.clone());
        self.persist_routes()?;
        Ok(())
    }

    pub fn remove(&mut self, host: &str) -> Result<bool> {
        let name = normalize_route_name(host);
        let removed = self.routes.remove(name.as_ref()).is_some();
        if removed {
            self.persist_routes()?;
        }
        Ok(removed)
    }

    fn persist_routes(&self) -> Result<()> {
        let mut doc = load_doc(&self.path)?;
        let routes = encode_routes(&self.routes);
        let routes_value = Value::try_from(routes).context("serialize routes")?;
        set_nested(&mut doc, &[AGENT_SECTION, ROUTES_SECTION], routes_value);
        write_doc_atomic(&self.path, &doc)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RouteSpecToml {
    local_addr: String,
    mode: TomlIngressMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TomlIngressMode {
    #[serde(rename = "http", alias = "http1")]
    Http,
    #[serde(rename = "http2", alias = "h2", alias = "grpc")]
    Http2,
    #[serde(rename = "tcp")]
    Tcp,
}

impl From<IngressMode> for TomlIngressMode {
    fn from(mode: IngressMode) -> Self {
        match mode {
            IngressMode::Http1 => TomlIngressMode::Http,
            IngressMode::Http2 => TomlIngressMode::Http2,
            IngressMode::Tcp => TomlIngressMode::Tcp,
        }
    }
}

impl From<TomlIngressMode> for IngressMode {
    fn from(mode: TomlIngressMode) -> Self {
        match mode {
            TomlIngressMode::Http => IngressMode::Http1,
            TomlIngressMode::Http2 => IngressMode::Http2,
            TomlIngressMode::Tcp => IngressMode::Tcp,
        }
    }
}

fn encode_routes(routes: &HashMap<String, RouteSpec>) -> HashMap<String, RouteSpecToml> {
    routes
        .iter()
        .map(|(host, route)| {
            (
                host.clone(),
                RouteSpecToml {
                    local_addr: route.local_addr.clone(),
                    mode: route.mode.into(),
                },
            )
        })
        .collect()
}

fn read_routes(doc: &Value) -> Result<HashMap<String, RouteSpec>> {
    let Some(value) = get_nested(doc, &[AGENT_SECTION, ROUTES_SECTION]) else {
        return Ok(HashMap::new());
    };

    let decoded: HashMap<String, RouteSpecToml> =
        value.clone().try_into().context("deserialize routes")?;

    Ok(decoded
        .into_iter()
        .map(|(host, route)| {
            (
                host,
                RouteSpec {
                    local_addr: route.local_addr,
                    mode: route.mode.into(),
                },
            )
        })
        .collect())
}

fn load_doc(path: &PathBuf) -> Result<Value> {
    if !path.exists() {
        return Ok(Value::Table(toml::value::Table::new()));
    }

    let content =
        fs::read_to_string(path).with_context(|| format!("read store {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(Value::Table(toml::value::Table::new()));
    }

    content
        .parse::<Value>()
        .with_context(|| format!("parse store {}", path.display()))
}

fn write_doc_atomic(path: &PathBuf, doc: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create store dir {}", parent.display()))?;
    }

    let content = toml::to_string_pretty(doc).context("serialize store")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let temp_path = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "default.toml".to_string()),
        stamp
    ));

    fs::write(&temp_path, content)
        .with_context(|| format!("write temp store {}", temp_path.display()))?;

    match fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(err) if path.exists() => {
            fs::remove_file(path)
                .with_context(|| format!("remove old store {}", path.display()))?;
            fs::rename(&temp_path, path)
                .with_context(|| format!("replace store {}", path.display()))?;
            Ok(())
        }
        Err(err) => Err(err).with_context(|| format!("replace store {}", path.display())),
    }
}

fn ensure_table(value: &mut Value) -> &mut toml::value::Table {
    if !matches!(value, Value::Table(_)) {
        *value = Value::Table(toml::value::Table::new());
    }
    match value {
        Value::Table(table) => table,
        _ => unreachable!(),
    }
}

fn set_nested(doc: &mut Value, path: &[&str], value: Value) {
    if path.is_empty() {
        return;
    }

    let mut current = ensure_table(doc);
    for key in &path[..path.len() - 1] {
        let entry = current
            .entry((*key).to_string())
            .or_insert_with(|| Value::Table(toml::value::Table::new()));
        current = ensure_table(entry);
    }
    current.insert(path[path.len() - 1].to_string(), value);
}

fn get_nested<'a>(doc: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = doc;
    for key in path {
        match current {
            Value::Table(table) => {
                current = table.get(*key)?;
            }
            _ => return None,
        }
    }
    Some(current)
}

fn store_path() -> PathBuf {
    data_path("default.toml")
}
