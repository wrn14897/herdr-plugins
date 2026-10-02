//! Readers for agent conversation history on local disk.
//!
//! A pane is mapped to its session through herdr's `agent_session` reference
//! when the agent's integration reports one, otherwise by the pane's cwd and
//! the most recently updated session there. Only user prompts and assistant
//! replies are kept; tool calls, tool output, and reasoning are skipped.
//! Everything is read locally; nothing leaves the machine.

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;
use serde_json::Value;

use crate::content::Role;

/// Herdr's `agent_session` reference on a pane.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionRef {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct Request {
    pub agent: String,
    pub cwd: String,
    pub session: Option<SessionRef>,
}

/// A located session and a cheap version stamp for change detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    File(PathBuf),
    OpenCode { db: PathBuf, session: String },
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub messages: usize,
    pub bytes: usize,
}

pub fn supported(agent: &str) -> bool {
    matches!(agent, "claude" | "codex" | "opencode")
}

/// Finds the session for `req` and returns it with its current version.
pub fn locate(req: &Request) -> Option<(Location, u64)> {
    match req.agent.as_str() {
        "claude" => file_location(claude::find(req)?),
        "codex" => file_location(codex::find(req)?),
        "opencode" => opencode::find(req),
        _ => None,
    }
}

/// Reads the newest messages of a located session, oldest first.
pub fn read(location: &Location, limits: Limits) -> Vec<(Role, String)> {
    let messages = match location {
        Location::File(path) if is_codex(path) => codex::read(path, limits),
        Location::File(path) => claude::read(path, limits),
        Location::OpenCode { db, session } => opencode::read(db, session, limits),
    };
    cap(messages, limits)
}

fn is_codex(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("rollout-"))
}

fn file_location(path: PathBuf) -> Option<(Location, u64)> {
    let version = mtime(&path)?;
    Some((Location::File(path), version))
}

fn mtime(path: &Path) -> Option<u64> {
    let modified = path.metadata().ok()?.modified().ok()?;
    let nanos = modified.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Keeps the newest messages within both limits.
fn cap(mut messages: Vec<(Role, String)>, limits: Limits) -> Vec<(Role, String)> {
    let mut bytes = 0;
    let mut keep = 0;
    for (_, text) in messages.iter().rev() {
        if keep == limits.messages || bytes + text.len() > limits.bytes {
            break;
        }
        bytes += text.len();
        keep += 1;
    }
    messages.drain(..messages.len() - keep);
    messages
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn env_dir(var: &str, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map_or_else(fallback, PathBuf::from)
}

/// Newest-modified file in `dir` whose name satisfies `pred`.
fn newest_file(dir: &Path, pred: impl Fn(&str) -> bool) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_str().is_some_and(&pred))
        .filter_map(|e| Some((mtime(&e.path())?, e.path())))
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

/// Opens a JSONL file positioned at most `max` bytes before its end, so huge
/// histories cost a bounded read. The first partial line is skipped.
fn tail_lines(path: &Path, max: u64) -> Option<impl Iterator<Item = String>> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut reader = BufReader::new(file);
    if start > 0 {
        let mut partial = Vec::new();
        reader.read_until(b'\n', &mut partial).ok()?;
    }
    Some(reader.lines().map_while(Result::ok))
}

/// How much of a JSONL history to read: generous, since tool output dominates.
fn tail_budget(limits: Limits) -> u64 {
    (limits.bytes as u64).saturating_mul(16).max(4 << 20)
}

mod claude {
    use super::{
        Limits, Request, Role, Value, env_dir, home, newest_file, tail_budget, tail_lines,
    };
    use std::path::{Path, PathBuf};

    fn projects() -> PathBuf {
        env_dir("CLAUDE_CONFIG_DIR", || home().join(".claude")).join("projects")
    }

    /// Claude Code stores sessions under a directory named after the cwd with
    /// every non-alphanumeric character replaced by `-`.
    pub(super) fn slug(cwd: &str) -> String {
        cwd.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }

    pub(super) fn find(req: &Request) -> Option<PathBuf> {
        let projects = projects();
        let dir = projects.join(slug(&req.cwd));
        if let Some(session) = &req.session {
            match session.kind.as_str() {
                "path" if Path::new(&session.value).is_file() => {
                    return Some(session.value.clone().into());
                }
                "id" => {
                    let name = format!("{}.jsonl", session.value);
                    let direct = dir.join(&name);
                    if direct.is_file() {
                        return Some(direct);
                    }
                    let found = std::fs::read_dir(&projects)
                        .ok()?
                        .filter_map(Result::ok)
                        .map(|e| e.path().join(&name))
                        .find(|p| p.is_file());
                    if found.is_some() {
                        return found;
                    }
                }
                _ => {}
            }
        }
        newest_file(&dir, |name| {
            Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
        })
    }

    pub(super) fn read(path: &Path, limits: Limits) -> Vec<(Role, String)> {
        let Some(lines) = tail_lines(path, tail_budget(limits)) else {
            return Vec::new();
        };
        lines.filter_map(|line| parse(&line)).collect()
    }

    pub(super) fn parse(line: &str) -> Option<(Role, String)> {
        let entry: Value = serde_json::from_str(line).ok()?;
        let role = match entry.get("type")?.as_str()? {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            _ => return None,
        };
        let flag = |key| entry.get(key).and_then(Value::as_bool).unwrap_or(false);
        if flag("isSidechain") || flag("isMeta") {
            return None;
        }
        let content = entry.pointer("/message/content")?;
        let text = match content {
            Value::String(s) => s.clone(),
            Value::Array(blocks) => blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => return None,
        };
        let text = text.trim();
        // Slash-command plumbing and interrupts are not conversation.
        let noise = text.starts_with("<command-")
            || text.starts_with("<local-command-")
            || text.starts_with("[Request interrupted");
        (!text.is_empty() && !noise).then(|| (role, text.to_owned()))
    }
}

mod codex {
    use super::{
        BufRead, BufReader, File, Limits, Request, Role, Value, env_dir, home, tail_budget,
        tail_lines,
    };
    use std::path::{Path, PathBuf};

    /// How many recent session files to check when matching by cwd.
    const SCAN_LIMIT: usize = 300;

    fn sessions() -> PathBuf {
        env_dir("CODEX_HOME", || home().join(".codex")).join("sessions")
    }

    /// Session files, newest first (paths are `YYYY/MM/DD/rollout-<time>-<id>.jsonl`).
    fn all_sessions() -> Vec<PathBuf> {
        let mut files = Vec::new();
        let mut stack = vec![sessions()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "jsonl") {
                    files.push(path);
                }
            }
        }
        files.sort_unstable_by(|a, b| b.cmp(a));
        files
    }

    pub(super) fn find(req: &Request) -> Option<PathBuf> {
        let files = all_sessions();
        if let Some(session) = req.session.as_ref().filter(|s| !s.value.is_empty()) {
            if session.kind == "path" && Path::new(&session.value).is_file() {
                return Some(session.value.clone().into());
            }
            let suffix = format!("{}.jsonl", session.value);
            if let Some(found) = files
                .iter()
                .find(|p| p.to_string_lossy().ends_with(&suffix))
            {
                return Some(found.clone());
            }
        }
        files
            .into_iter()
            .take(SCAN_LIMIT)
            .find(|p| session_cwd(p).as_deref() == Some(req.cwd.as_str()))
    }

    fn session_cwd(path: &Path) -> Option<String> {
        let mut first = String::new();
        BufReader::new(File::open(path).ok()?)
            .read_line(&mut first)
            .ok()?;
        let meta: Value = serde_json::from_str(&first).ok()?;
        meta.pointer("/payload/cwd")?.as_str().map(str::to_owned)
    }

    pub(super) fn read(path: &Path, limits: Limits) -> Vec<(Role, String)> {
        let Some(lines) = tail_lines(path, tail_budget(limits)) else {
            return Vec::new();
        };
        lines.filter_map(|line| parse(&line)).collect()
    }

    pub(super) fn parse(line: &str) -> Option<(Role, String)> {
        let entry: Value = serde_json::from_str(line).ok()?;
        if entry.get("type")?.as_str()? != "event_msg" {
            return None;
        }
        let payload = entry.get("payload")?;
        let role = match payload.get("type")?.as_str()? {
            "user_message" => Role::User,
            "agent_message" => Role::Assistant,
            _ => return None,
        };
        let text = payload.get("message")?.as_str()?.trim();
        (!text.is_empty()).then(|| (role, text.to_owned()))
    }
}

mod opencode {
    use super::{
        Connection, Limits, Location, OpenFlags, OptionalExtension, Request, Role, Value, env_dir,
        home,
    };
    use std::path::{Path, PathBuf};

    fn database() -> PathBuf {
        env_dir("XDG_DATA_HOME", || home().join(".local/share")).join("opencode/opencode.db")
    }

    fn open(db: &Path) -> Option<Connection> {
        let conn = Connection::open_with_flags(
            db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()?;
        // OpenCode may be writing; wait briefly instead of failing.
        conn.busy_timeout(std::time::Duration::from_millis(500))
            .ok()?;
        Some(conn)
    }

    pub(super) fn find(req: &Request) -> Option<(Location, u64)> {
        find_in(&database(), req)
    }

    pub(super) fn find_in(db: &Path, req: &Request) -> Option<(Location, u64)> {
        if !db.is_file() {
            return None;
        }
        let conn = open(db)?;
        let by_id = req
            .session
            .as_ref()
            .filter(|s| s.kind == "id" && !s.value.is_empty())
            .and_then(|s| {
                conn.query_row(
                    "SELECT id, time_updated FROM session WHERE id = ?1",
                    [&s.value],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()
                .ok()
                .flatten()
            });
        let (session, updated) = match by_id {
            Some(found) => found,
            None => conn
                .query_row(
                    "SELECT id, time_updated FROM session
                     WHERE directory = ?1 AND parent_id IS NULL AND time_archived IS NULL
                     ORDER BY time_updated DESC LIMIT 1",
                    [&req.cwd],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()
                .ok()??,
        };
        let version = u64::try_from(updated).unwrap_or_default();
        Some((
            Location::OpenCode {
                db: db.to_path_buf(),
                session,
            },
            version,
        ))
    }

    pub(super) fn read(db: &Path, session: &str, limits: Limits) -> Vec<(Role, String)> {
        let Some(conn) = open(db) else {
            return Vec::new();
        };
        // Text parts are a minority among tool/step parts, so over-fetch.
        let fetch = i64::try_from(limits.messages.saturating_mul(8)).unwrap_or(i64::MAX);
        let Ok(mut stmt) = conn.prepare(
            "SELECT m.data, p.data FROM part p JOIN message m ON m.id = p.message_id
             WHERE p.session_id = ?1
             ORDER BY p.time_created DESC, p.id DESC LIMIT ?2",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map(rusqlite::params![session, fetch], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        }) else {
            return Vec::new();
        };
        let mut messages: Vec<(Role, String)> = rows
            .filter_map(Result::ok)
            .filter_map(|(message, part)| parse(&message, &part))
            .collect();
        messages.reverse();
        messages
    }

    pub(super) fn parse(message: &str, part: &str) -> Option<(Role, String)> {
        let part: Value = serde_json::from_str(part).ok()?;
        if part.get("type")?.as_str()? != "text"
            || part.get("synthetic").and_then(Value::as_bool) == Some(true)
        {
            return None;
        }
        let message: Value = serde_json::from_str(message).ok()?;
        let role = match message.get("role")?.as_str()? {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            _ => return None,
        };
        let text = part.get("text")?.as_str()?.trim();
        (!text.is_empty()).then(|| (role, text.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("herdr-picker-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn texts(messages: &[(Role, String)]) -> Vec<(Role, &str)> {
        messages.iter().map(|(r, t)| (*r, t.as_str())).collect()
    }

    #[test]
    fn claude_slug_replaces_non_alphanumerics() {
        assert_eq!(
            claude::slug("/Users/w/.config/nvim"),
            "-Users-w--config-nvim"
        );
        assert_eq!(claude::slug("/a/b_c-d"), "-a-b-c-d");
    }

    #[test]
    fn claude_keeps_prompts_and_replies_only() {
        let lines = [
            r#"{"type":"mode","mode":"normal"}"#,
            r#"{"type":"user","message":{"role":"user","content":"fix the login bug"}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hmm"},{"type":"tool_use","name":"Bash"}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Fixed it."}]}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"meta"}}"#,
            r#"{"type":"user","isSidechain":true,"message":{"content":"side"}}"#,
            r#"{"type":"user","message":{"content":"<command-name>/clear</command-name>"}}"#,
        ];
        let parsed: Vec<_> = lines.iter().filter_map(|l| claude::parse(l)).collect();
        assert_eq!(
            texts(&parsed),
            [
                (Role::User, "fix the login bug"),
                (Role::Assistant, "Fixed it.")
            ]
        );
    }

    #[test]
    fn codex_reads_event_messages() {
        let lines = [
            r#"{"type":"session_meta","payload":{"cwd":"/x"}}"#,
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"why is it slow?"}}"#,
            r#"{"type":"response_item","payload":{"type":"message","role":"assistant"}}"#,
            r#"{"type":"event_msg","payload":{"type":"token_count"}}"#,
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"An N+1 query."}}"#,
        ];
        let parsed: Vec<_> = lines.iter().filter_map(|l| codex::parse(l)).collect();
        assert_eq!(
            texts(&parsed),
            [
                (Role::User, "why is it slow?"),
                (Role::Assistant, "An N+1 query.")
            ]
        );
    }

    #[test]
    fn jsonl_tail_skips_partial_first_line() {
        let dir = temp_dir("tail");
        let path = dir.join("s.jsonl");
        let mut f = File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"user","message":{{"content":"{}"}}}}"#,
            "x".repeat(200)
        )
        .unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"content":"last"}}}}"#).unwrap();
        drop(f);
        let lines: Vec<String> = tail_lines(&path, 60).unwrap().collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("last"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cap_keeps_newest_within_limits() {
        let messages: Vec<(Role, String)> = (0..10)
            .map(|i| (Role::User, format!("message {i}")))
            .collect();
        let kept = cap(
            messages.clone(),
            Limits {
                messages: 3,
                bytes: 1 << 20,
            },
        );
        assert_eq!(kept.last().unwrap().1, "message 9");
        assert_eq!(kept.len(), 3);
        let kept = cap(
            messages,
            Limits {
                messages: 100,
                bytes: 20,
            },
        );
        assert_eq!(kept.len(), 2, "two 9-byte messages fit in 20 bytes");
    }

    #[test]
    fn opencode_reads_text_parts_of_newest_session() {
        let dir = temp_dir("opencode");
        let db = dir.join("opencode.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT NOT NULL,
                time_updated INTEGER NOT NULL, time_archived INTEGER);
            CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT);
            CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT,
                time_created INTEGER, data TEXT);
            INSERT INTO session VALUES ('old', NULL, '/proj', 1, NULL), ('new', NULL, '/proj', 5, NULL),
                ('child', 'new', '/proj', 9, NULL);
            INSERT INTO message VALUES ('m1', 'new', '{"role":"user"}'), ('m2', 'new', '{"role":"assistant"}');
            INSERT INTO part VALUES
                ('p1', 'm1', 'new', 1, '{"type":"text","text":"add retries"}'),
                ('p2', 'm1', 'new', 2, '{"type":"text","text":"ctx","synthetic":true}'),
                ('p3', 'm2', 'new', 3, '{"type":"tool","tool":"bash"}'),
                ('p4', 'm2', 'new', 4, '{"type":"text","text":"Added exponential backoff."}');
            "#,
        )
        .unwrap();
        drop(conn);

        let req = Request {
            agent: "opencode".into(),
            cwd: "/proj".into(),
            session: None,
        };
        let (location, version) = opencode::find_in(&db, &req).unwrap();
        assert_eq!(
            location,
            Location::OpenCode {
                db: db.clone(),
                session: "new".into()
            }
        );
        assert_eq!(version, 5);
        let messages = read(
            &location,
            Limits {
                messages: 50,
                bytes: 1 << 20,
            },
        );
        assert_eq!(
            texts(&messages),
            [
                (Role::User, "add retries"),
                (Role::Assistant, "Added exponential backoff.")
            ]
        );

        let by_id = Request {
            session: Some(SessionRef {
                kind: "id".into(),
                value: "old".into(),
            }),
            ..req
        };
        let (location, _) = opencode::find_in(&db, &by_id).unwrap();
        assert_eq!(
            location,
            Location::OpenCode {
                db: db.clone(),
                session: "old".into()
            }
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
