//! Minimal Model Context Protocol client (stdio and streamable HTTP) that exposes
//! server tools as Ecliven tools named `mcp__<server>__<tool>`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{oneshot, Mutex};

use crate::config::McpServerConfig;
use crate::tools::{Tool, ToolContext, ToolError, ToolOutput, ToolRisk};

const PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_STARTUP_TIMEOUT_SECS: u64 = 20;
const CALL_TIMEOUT_SECS: u64 = 300;

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

enum Transport {
    Stdio {
        stdin: Box<Mutex<ChildStdin>>,
        pending: Pending,
        _child: Box<Mutex<Child>>,
    },
    Http {
        client: reqwest::Client,
        url: String,
        headers: HashMap<String, String>,
        session_id: Mutex<Option<String>>,
    },
}

pub struct McpClient {
    pub server: String,
    transport: Transport,
    next_id: AtomicU64,
}

fn rpc_result(msg: &Value) -> Result<Value, String> {
    if let Some(err) = msg.get("error") {
        let text = err
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(text.to_string());
    }
    Ok(msg.get("result").cloned().unwrap_or(Value::Null))
}

/// Extracts the JSON-RPC response with `id` from a JSON or SSE body.
fn parse_http_body(body: &str, id: u64) -> Result<Value, String> {
    let candidates: Vec<Value> =
        if body.trim_start().starts_with('{') || body.trim_start().starts_with('[') {
            match serde_json::from_str::<Value>(body) {
                Ok(Value::Array(items)) => items,
                Ok(v) => vec![v],
                Err(e) => return Err(format!("invalid JSON from MCP server: {e}")),
            }
        } else {
            body.lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .filter_map(|d| serde_json::from_str::<Value>(d.trim()).ok())
                .collect()
        };
    candidates
        .iter()
        .find(|m| m.get("id").and_then(Value::as_u64) == Some(id))
        .map(rpc_result)
        .unwrap_or_else(|| Err("MCP server did not return a response".into()))
}

impl McpClient {
    pub async fn connect(cfg: &McpServerConfig) -> Result<Self, String> {
        let transport = if let Some(command) = &cfg.command {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(&cfg.args)
                .envs(&cfg.env)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let mut child = cmd
                .spawn()
                .map_err(|e| format!("failed to start '{command}': {e}"))?;
            let stdin = child.stdin.take().ok_or("no stdin")?;
            let stdout = child.stdout.take().ok_or("no stdout")?;
            let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
            let reader_pending = Arc::clone(&pending);
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if msg.get("method").is_some() {
                        continue;
                    }
                    if let Some(id) = msg.get("id").and_then(Value::as_u64) {
                        if let Some(tx) = reader_pending.lock().await.remove(&id) {
                            let _ = tx.send(rpc_result(&msg));
                        }
                    }
                }
                for (_, tx) in reader_pending.lock().await.drain() {
                    let _ = tx.send(Err("MCP server exited".into()));
                }
            });
            Transport::Stdio {
                stdin: Box::new(Mutex::new(stdin)),
                pending,
                _child: Box::new(Mutex::new(child)),
            }
        } else if let Some(url) = &cfg.url {
            Transport::Http {
                client: reqwest::Client::new(),
                url: url.clone(),
                headers: cfg.headers.clone(),
                session_id: Mutex::new(None),
            }
        } else {
            return Err("needs either `command` or `url`".into());
        };

        let client = Self {
            server: cfg.name.clone(),
            transport,
            next_id: AtomicU64::new(1),
        };
        let timeout = Duration::from_secs(
            cfg.startup_timeout_secs
                .unwrap_or(DEFAULT_STARTUP_TIMEOUT_SECS),
        );
        tokio::time::timeout(
            timeout,
            client.request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "ecliven-code", "version": env!("CARGO_PKG_VERSION") }
                }),
            ),
        )
        .await
        .map_err(|_| format!("initialize timed out after {}s", timeout.as_secs()))??;
        client.notify("notifications/initialized").await?;
        Ok(client)
    }

    async fn notify(&self, method: &str) -> Result<(), String> {
        let msg = json!({ "jsonrpc": "2.0", "method": method });
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let mut w = stdin.lock().await;
                w.write_all(format!("{msg}\n").as_bytes())
                    .await
                    .map_err(|e| e.to_string())
            }
            Transport::Http { .. } => self.post(&msg).await.map(|_| ()),
        }
    }

    async fn post(&self, msg: &Value) -> Result<String, String> {
        let Transport::Http {
            client,
            url,
            headers,
            session_id,
        } = &self.transport
        else {
            unreachable!("post is only used by the HTTP transport");
        };
        let mut req = client
            .post(url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .json(msg);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        if let Some(sid) = session_id.lock().await.clone() {
            req = req.header("Mcp-Session-Id", sid);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        if let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            *session_id.lock().await = Some(sid.to_string());
        }
        let status = resp.status();
        let body = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "HTTP {status}: {}",
                body.chars().take(300).collect::<String>()
            ));
        }
        Ok(body)
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        match &self.transport {
            Transport::Stdio { stdin, pending, .. } => {
                let (tx, rx) = oneshot::channel();
                pending.lock().await.insert(id, tx);
                {
                    let mut w = stdin.lock().await;
                    w.write_all(format!("{msg}\n").as_bytes())
                        .await
                        .map_err(|e| e.to_string())?;
                    w.flush().await.map_err(|e| e.to_string())?;
                }
                tokio::time::timeout(Duration::from_secs(CALL_TIMEOUT_SECS), rx)
                    .await
                    .map_err(|_| format!("'{method}' timed out"))?
                    .map_err(|_| "MCP server closed the connection".to_string())?
            }
            Transport::Http { .. } => parse_http_body(&self.post(&msg).await?, id),
        }
    }

    pub async fn list_tools(&self) -> Result<Vec<Value>, String> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let result = self.request("tools/list", params).await?;
            if let Some(items) = result.get("tools").and_then(Value::as_array) {
                tools.extend(items.iter().cloned());
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
    }
}

pub fn render_call_result(result: &Value) -> (String, bool) {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut parts = Vec::new();
    for item in result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => parts.push(
                item.get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ),
            Some("resource") => parts.push(
                item.pointer("/resource/text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| "[resource]".into()),
            ),
            Some(other) => parts.push(format!("[{other} content]")),
            None => {}
        }
    }
    if parts.is_empty() {
        if let Some(structured) = result.get("structuredContent") {
            parts.push(structured.to_string());
        }
    }
    (parts.join("\n"), is_error)
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub struct McpTool {
    client: Arc<McpClient>,
    remote_name: String,
    full_name: String,
    description: String,
    schema: Value,
    trusted: bool,
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.schema.clone()
    }

    fn risk_level(&self) -> ToolRisk {
        if self.trusted {
            ToolRisk::ReadOnly
        } else {
            ToolRisk::ShellExec
        }
    }

    async fn execute(&self, _ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let arguments = if args.is_object() {
            args.clone()
        } else {
            json!({})
        };
        let result = self
            .client
            .request(
                "tools/call",
                json!({ "name": self.remote_name, "arguments": arguments }),
            )
            .await
            .map_err(ToolError::Execution)?;
        let (text, is_error) = render_call_result(&result);
        Ok(if is_error {
            ToolOutput::err(text)
        } else {
            ToolOutput::ok(text)
        })
    }
}

#[derive(Default)]
pub struct McpConnections {
    pub tools: Vec<Arc<dyn Tool>>,
    /// (server, number of tools) for connected servers.
    pub servers: Vec<(String, usize)>,
    pub errors: Vec<String>,
    clients: Vec<Arc<McpClient>>,
}

impl std::fmt::Debug for McpConnections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConnections")
            .field("servers", &self.servers)
            .field("errors", &self.errors)
            .finish()
    }
}

impl McpConnections {
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        for (name, count) in &self.servers {
            lines.push(format!("  ✓ {name}: {count} tools"));
        }
        for err in &self.errors {
            lines.push(format!("  ✗ {err}"));
        }
        if lines.is_empty() {
            "No MCP servers configured. Add [mcp_servers.<name>] to .ecliven.toml.".into()
        } else {
            format!("MCP servers:\n{}", lines.join("\n"))
        }
    }

    pub fn client_count(&self) -> usize {
        self.clients.len()
    }
}

/// Connects to every enabled server concurrently; failures are reported, not fatal.
pub async fn connect_all(servers: &[McpServerConfig]) -> McpConnections {
    let attempts = servers.iter().filter(|s| s.enabled).map(|cfg| async move {
        let client = Arc::new(McpClient::connect(cfg).await?);
        let tools = client.list_tools().await?;
        Ok::<_, String>((cfg.clone(), client, tools))
    });
    let mut out = McpConnections::default();
    for (cfg, result) in servers
        .iter()
        .filter(|s| s.enabled)
        .zip(futures::future::join_all(attempts).await)
    {
        match result {
            Ok((cfg, client, tools)) => {
                for t in &tools {
                    let Some(remote) = t.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    let schema = t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
                    out.tools.push(Arc::new(McpTool {
                        client: Arc::clone(&client),
                        remote_name: remote.to_string(),
                        full_name: format!("mcp__{}__{}", sanitize(&cfg.name), sanitize(remote)),
                        description: t
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("MCP tool")
                            .to_string(),
                        schema,
                        trusted: cfg.trusted,
                    }));
                }
                out.servers.push((cfg.name.clone(), tools.len()));
                out.clients.push(client);
            }
            Err(err) => out.errors.push(format!("{}: {err}", cfg.name)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_and_sse_bodies() {
        let json_body = r#"{"jsonrpc":"2.0","id":3,"result":{"ok":true}}"#;
        assert_eq!(parse_http_body(json_body, 3).unwrap()["ok"], true);
        let sse = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":4,\"error\":{\"message\":\"nope\"}}\n\n";
        assert_eq!(parse_http_body(sse, 4).unwrap_err(), "nope");
    }

    #[test]
    fn renders_text_content_and_error_flag() {
        let (text, err) = render_call_result(&json!({
            "content": [{ "type": "text", "text": "a" }, { "type": "image", "data": "x" }],
            "isError": true
        }));
        assert_eq!(text, "a\n[image content]");
        assert!(err);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stdio_server_lists_and_calls_tools() {
        let script = r#"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"initialize"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"protocolVersion":"2025-06-18","capabilities":{}}}\n' "$id";;
    *'"tools/list"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"echo.say","description":"Echo","inputSchema":{"type":"object"}}]}}\n' "$id";;
    *'"tools/call"'*) printf '{"jsonrpc":"2.0","id":%s,"result":{"content":[{"type":"text","text":"pong"}]}}\n' "$id";;
  esac
done
"#;
        let cfg = McpServerConfig {
            name: "demo".into(),
            command: Some("sh".into()),
            args: vec!["-c".into(), script.into()],
            enabled: true,
            ..McpServerConfig::default()
        };
        let conns = connect_all(&[cfg]).await;
        assert!(conns.errors.is_empty(), "{:?}", conns.errors);
        assert_eq!(conns.tools.len(), 1);
        let tool = &conns.tools[0];
        assert_eq!(tool.name(), "mcp__demo__echo_say");
        assert_eq!(tool.risk_level(), ToolRisk::ShellExec);
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tool
            .execute(&ToolContext::new(tmp.path().to_path_buf(), 5), &json!({}))
            .await
            .unwrap();
        assert_eq!(out.content, "pong");
    }
}
