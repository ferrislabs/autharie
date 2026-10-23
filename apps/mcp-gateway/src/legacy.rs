use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use reqwest::{RequestBuilder, Response, StatusCode, header};
use serde_json::{Value, json};
use tokio::sync::Mutex as AsyncMutex;

pub const SESSION_TTL: Duration = Duration::from_secs(600);
pub const MAX_SESSIONS: usize = 10_000;
const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const SESSION_HEADER: &str = "mcp-session-id";
const PROTOCOL_HEADER: &str = "mcp-protocol-version";
const INITIALIZE_ID: i64 = 0;

#[derive(Debug, thiserror::Error)]
pub enum LegacyError {
    #[error("upstream refused by the egress rule")]
    Refused,
    #[error("upstream call failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("upstream answered {0}")]
    Status(StatusCode),
    #[error("upstream answer is not a JSON-RPC response")]
    Malformed,
    #[error("upstream answer is too large")]
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKey {
    subject: String,
    agent: String,
    resource: String,
}

impl SessionKey {
    pub fn new(subject: &str, agent: &str, resource: &str) -> Self {
        Self {
            subject: subject.to_owned(),
            agent: agent.to_owned(),
            resource: resource.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    id: Option<String>,
}

impl Session {
    pub fn new(id: Option<String>) -> Self {
        Self { id }
    }
}

type Cell = Arc<AsyncMutex<Option<Session>>>;

struct Slot {
    cell: Cell,
    last_used: Instant,
}

pub struct Sessions {
    slots: Mutex<HashMap<SessionKey, Slot>>,
    ttl: Duration,
    capacity: usize,
}

impl Default for Sessions {
    fn default() -> Self {
        Self::new(SESSION_TTL, MAX_SESSIONS)
    }
}

impl Sessions {
    pub fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            ttl,
            capacity: capacity.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<SessionKey, Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn cell(&self, key: &SessionKey, now: Instant) -> Cell {
        let mut slots = self.lock();
        let ttl = self.ttl;
        if let Some(slot) = slots.get_mut(key)
            && now.saturating_duration_since(slot.last_used) <= ttl
        {
            slot.last_used = now;
            return Arc::clone(&slot.cell);
        }
        slots.remove(key);
        if slots.len() >= self.capacity {
            slots.retain(|_, slot| now.saturating_duration_since(slot.last_used) <= ttl);
        }
        while slots.len() >= self.capacity {
            let Some(oldest) = slots
                .iter()
                .min_by_key(|(_, slot)| slot.last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            slots.remove(&oldest);
        }
        let cell = Cell::default();
        slots.insert(
            key.clone(),
            Slot {
                cell: Arc::clone(&cell),
                last_used: now,
            },
        );
        cell
    }

    pub async fn session<E, F, Fut>(
        &self,
        key: &SessionKey,
        stale: Option<&Session>,
        open: F,
    ) -> Result<Session, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Session, E>>,
    {
        let cell = self.cell(key, Instant::now());
        let mut current = cell.lock().await;
        if let Some(session) = current.as_ref()
            && Some(session) != stale
        {
            return Ok(session.clone());
        }
        let opened = open().await?;
        *current = Some(opened.clone());
        Ok(opened)
    }
}

pub struct Wire<'a, R> {
    pub request: R,
    pub version: &'a str,
    pub token: &'a str,
    pub request_id: &'a str,
}

impl<R> Wire<'_, R>
where
    R: Fn() -> Result<RequestBuilder, LegacyError>,
{
    pub async fn call(
        &self,
        sessions: &Sessions,
        key: &SessionKey,
        message: &Value,
    ) -> Result<Value, LegacyError> {
        let id = message.get("id").unwrap_or(&Value::Null);
        let session = sessions.session(key, None, || self.open()).await?;
        match self.exchange(&session, message, id).await {
            Err(LegacyError::Status(StatusCode::NOT_FOUND)) => {
                let fresh = sessions
                    .session(key, Some(&session), || self.open())
                    .await?;
                self.exchange(&fresh, message, id).await
            }
            other => other,
        }
    }

    async fn open(&self) -> Result<Session, LegacyError> {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": INITIALIZE_ID,
            "method": "initialize",
            "params": {
                "protocolVersion": self.version,
                "capabilities": {},
                "clientInfo": {
                    "name": "autharie-mcp-gateway",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            },
        });
        let response = self.send(&initialize, None).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(LegacyError::Status(status));
        }
        let id = response
            .headers()
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let answer = read_message(response, &json!(INITIALIZE_ID)).await?;
        if answer.get("result").is_none() {
            return Err(LegacyError::Malformed);
        }
        let session = Session::new(id);
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let ack = self.send(&initialized, Some(&session)).await?;
        if !ack.status().is_success() {
            return Err(LegacyError::Status(ack.status()));
        }
        Ok(session)
    }

    async fn exchange(
        &self,
        session: &Session,
        message: &Value,
        id: &Value,
    ) -> Result<Value, LegacyError> {
        let response = self.send(message, Some(session)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(LegacyError::Status(status));
        }
        read_message(response, id).await
    }

    async fn send(&self, body: &Value, session: Option<&Session>) -> Result<Response, LegacyError> {
        let mut token = header::HeaderValue::from_str(&format!("Bearer {}", self.token))
            .map_err(|_| LegacyError::Refused)?;
        token.set_sensitive(true);
        let mut request = (self.request)()?
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header(header::AUTHORIZATION, token)
            .header("x-request-id", self.request_id);
        if let Some(session) = session {
            request = request.header(PROTOCOL_HEADER, self.version);
            if let Some(id) = &session.id {
                request = request.header(SESSION_HEADER, id);
            }
        }
        Ok(request.body(body.to_string()).send().await?)
    }
}

async fn read_message(mut response: Response, id: &Value) -> Result<Value, LegacyError> {
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(LegacyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    if content_type.starts_with("application/json") {
        serde_json::from_slice::<Value>(&body)
            .ok()
            .filter(Value::is_object)
            .ok_or(LegacyError::Malformed)
    } else if content_type.starts_with("text/event-stream") {
        std::str::from_utf8(&body)
            .ok()
            .and_then(|text| sse_response(text, id))
            .ok_or(LegacyError::Malformed)
    } else {
        Err(LegacyError::Malformed)
    }
}

fn sse_response(text: &str, id: &Value) -> Option<Value> {
    text.replace("\r\n", "\n").split("\n\n").find_map(|block| {
        let data = block
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|rest| rest.strip_prefix(' ').unwrap_or(rest))
            .collect::<Vec<_>>()
            .join("\n");
        let message: Value = serde_json::from_str(&data).ok()?;
        let answers = message.get("result").is_some() || message.get("error").is_some();
        (answers && message.get("id") == Some(id)).then_some(message)
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn key(n: usize) -> SessionKey {
        SessionKey::new(&format!("s{n}"), "agent", "https://x.example/mcp/crm")
    }

    fn ready(id: &str) -> Session {
        Session::new(Some(id.to_owned()))
    }

    #[test]
    fn an_idle_session_is_replaced_after_the_ttl_and_a_used_one_is_kept() {
        let sessions = Sessions::new(Duration::from_secs(600), 10);
        let start = Instant::now();
        let first = sessions.cell(&key(1), start);
        let kept = sessions.cell(&key(1), start + Duration::from_secs(500));
        assert!(Arc::ptr_eq(&first, &kept));
        let still = sessions.cell(&key(1), start + Duration::from_secs(1000));
        assert!(Arc::ptr_eq(&first, &still));
        let replaced = sessions.cell(&key(1), start + Duration::from_secs(1700));
        assert!(!Arc::ptr_eq(&first, &replaced));
        assert_eq!(sessions.len(), 1);
    }

    #[test]
    fn the_map_never_outgrows_its_bound_and_drops_the_oldest() {
        let sessions = Sessions::new(Duration::from_secs(600), 3);
        let start = Instant::now();
        let oldest = sessions.cell(&key(0), start);
        for n in 1..=5 {
            sessions.cell(&key(n), start + Duration::from_secs(n as u64));
            assert!(sessions.len() <= 3);
        }
        assert_eq!(sessions.len(), 3);
        let again = sessions.cell(&key(0), start + Duration::from_secs(6));
        assert!(!Arc::ptr_eq(&oldest, &again));
    }

    #[test]
    fn expired_sessions_are_purged_before_a_live_one_is_evicted() {
        let sessions = Sessions::new(Duration::from_secs(10), 2);
        let start = Instant::now();
        sessions.cell(&key(0), start);
        let live = sessions.cell(&key(1), start + Duration::from_secs(15));
        sessions.cell(&key(2), start + Duration::from_secs(16));
        let same = sessions.cell(&key(1), start + Duration::from_secs(17));
        assert!(Arc::ptr_eq(&live, &same));
    }

    #[tokio::test]
    async fn concurrent_first_calls_open_one_session() {
        let sessions = Arc::new(Sessions::default());
        let opened = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let sessions = Arc::clone(&sessions);
                let opened = Arc::clone(&opened);
                tokio::spawn(async move {
                    sessions
                        .session(&key(1), None, || async {
                            tokio::time::sleep(Duration::from_millis(30)).await;
                            opened.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, ()>(ready("a"))
                        })
                        .await
                })
            })
            .collect();
        for task in tasks {
            assert_eq!(task.await.unwrap(), Ok(ready("a")));
        }
        assert_eq!(opened.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_stale_session_is_reopened_once_however_many_callers_saw_it_fail() {
        let sessions = Arc::new(Sessions::default());
        let opened = Arc::new(AtomicUsize::new(0));
        let open = |label: &'static str| {
            let opened = Arc::clone(&opened);
            move || async move {
                opened.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(ready(label))
            }
        };
        let first = sessions.session(&key(1), None, open("a")).await.unwrap();
        let second = sessions
            .session(&key(1), Some(&first), open("b"))
            .await
            .unwrap();
        let third = sessions
            .session(&key(1), Some(&first), open("c"))
            .await
            .unwrap();
        assert_eq!((second, third), (ready("b"), ready("b")));
        assert_eq!(opened.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_open_is_not_remembered() {
        let sessions = Sessions::default();
        let failed = sessions
            .session(&key(1), None, || async { Err::<Session, _>("down") })
            .await;
        assert_eq!(failed, Err("down"));
        let ok = sessions
            .session(&key(1), None, || async { Ok::<_, &str>(ready("a")) })
            .await;
        assert_eq!(ok, Ok(ready("a")));
    }

    #[test]
    fn the_answer_to_the_request_is_picked_out_of_an_event_stream() {
        let text = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
                    event: message\r\ndata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":4,\"result\":{}}\r\n\r\n";
        assert_eq!(
            sse_response(text, &json!(4)),
            Some(json!({"jsonrpc":"2.0","id":4,"result":{}}))
        );
        assert_eq!(sse_response(text, &json!(5)), None);
        assert_eq!(sse_response("data: nope\n\n", &json!(4)), None);
    }
}
