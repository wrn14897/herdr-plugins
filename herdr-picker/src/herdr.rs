//! Minimal client for the herdr socket API.
//!
//! Herdr speaks newline-delimited JSON over a Unix socket and closes the
//! connection after each response, so every request opens a fresh connection.
//! A round trip is well under a millisecond, far cheaper than spawning the CLI.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::model::Snapshot;

const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone)]
pub struct Client {
    socket: PathBuf,
}

impl Client {
    pub fn from_env() -> Self {
        let socket =
            std::env::var_os("HERDR_SOCKET_PATH").map_or_else(default_socket, PathBuf::from);
        Self { socket }
    }

    pub fn request(&self, method: &str, params: &Value) -> Result<Value> {
        let mut stream = UnixStream::connect(&self.socket)
            .with_context(|| format!("connect to herdr socket {}", self.socket.display()))?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;

        let mut line =
            serde_json::to_vec(&json!({ "id": "picker", "method": method, "params": params }))?;
        line.push(b'\n');
        stream.write_all(&line)?;
        stream.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response)?;
        if response.is_empty() {
            bail!("herdr closed the connection without a response to {method}");
        }

        let mut value: Value = serde_json::from_str(&response)?;
        if let Some(err) = value.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            return Err(anyhow!("{method}: {msg}"));
        }
        value
            .get_mut("result")
            .map(Value::take)
            .ok_or_else(|| anyhow!("{method}: response has no result"))
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        let mut result = self.request("session.snapshot", &json!({}))?;
        let snapshot = result
            .get_mut("snapshot")
            .map(Value::take)
            .ok_or_else(|| anyhow!("session.snapshot: missing snapshot"))?;
        Ok(serde_json::from_value(snapshot)?)
    }

    /// Visible screen of a pane, with ANSI styling preserved.
    pub fn read_visible(&self, pane_id: &str) -> Result<String> {
        let result = self.request(
            "pane.read",
            &json!({ "pane_id": pane_id, "source": "visible", "format": "ansi", "strip_ansi": false }),
        )?;
        result
            .pointer("/read/text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("pane.read: missing text"))
    }

    /// Recent scrollback of a pane as plain text, soft wraps joined.
    pub fn read_recent(&self, pane_id: &str, lines: u32) -> Result<String> {
        let result = self.request(
            "pane.read",
            &json!({ "pane_id": pane_id, "source": "recent_unwrapped", "format": "text", "lines": lines }),
        )?;
        result
            .pointer("/read/text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| anyhow!("pane.read: missing text"))
    }

    /// Focuses the agent pane (switching workspace and tab as needed).
    pub fn focus_agent(&self, pane_id: &str) -> Result<()> {
        self.request("agent.focus", &json!({ "target": pane_id }))?;
        Ok(())
    }

    pub fn focus_workspace(&self, workspace_id: &str) -> Result<()> {
        self.request("workspace.focus", &json!({ "workspace_id": workspace_id }))?;
        Ok(())
    }
}

fn default_socket() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_default();
    match std::env::var("HERDR_SESSION") {
        Ok(name) if !name.is_empty() && name != "default" => {
            config.join("herdr/sessions").join(name).join("herdr.sock")
        }
        _ => config.join("herdr/herdr.sock"),
    }
}
