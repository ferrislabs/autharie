use http::HeaderMap;
use serde_json::{Map, Value, json};

pub const CODE_PARSE_ERROR: i64 = -32700;
pub const CODE_INVALID_REQUEST: i64 = -32600;
pub const CODE_INVALID_PARAMS: i64 = -32602;
pub const CODE_DENIED: i64 = -32001;
pub const CODE_AGENT_REFUSED: i64 = -32003;
pub const CODE_HEADER_MISMATCH: i64 = -32020;

pub const MCP_METHOD: &str = "mcp-method";
pub const MCP_NAME: &str = "mcp-name";

pub const PROTOCOL_2026: &str = "2026-07-28";
pub const LEGACY_PROTOCOLS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
pub const NEWEST_LEGACY: &str = "2025-11-25";

const META_PREFIX: &str = "io.modelcontextprotocol/";
const META_PROTOCOL: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

pub fn negotiate(requested: Option<&str>) -> &'static str {
    requested
        .and_then(|v| LEGACY_PROTOCOLS.iter().find(|known| **known == v))
        .copied()
        .unwrap_or(NEWEST_LEGACY)
}

pub fn is_legacy_protocol(version: &str) -> bool {
    LEGACY_PROTOCOLS.contains(&version)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    CallTool,
    ReadResource,
    GetPrompt,
}

impl Operation {
    pub fn action(self) -> &'static str {
        match self {
            Self::CallTool => "call_tool",
            Self::ReadResource => "read_resource",
            Self::GetPrompt => "get_prompt",
        }
    }

    fn from_method(method: &str) -> Option<Self> {
        match method {
            "tools/call" => Some(Self::CallTool),
            "resources/read" => Some(Self::ReadResource),
            "prompts/get" => Some(Self::GetPrompt),
            _ => None,
        }
    }

    fn name_field(self) -> &'static str {
        match self {
            Self::ReadResource => "uri",
            Self::CallTool | Self::GetPrompt => "name",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    pub operation: Operation,
    pub name: String,
    pub arguments: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub id: Value,
    pub method: String,
    pub target: Option<Target>,
    pub requested_protocol: Option<String>,
}

impl Message {
    pub fn is_initialize(&self) -> bool {
        self.method == "initialize"
    }

    pub fn is_notification(&self) -> bool {
        self.method.starts_with("notifications/")
    }

    pub fn is_tools_list(&self) -> bool {
        self.method == "tools/list"
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rejection {
    pub id: Value,
    pub code: i64,
    pub message: &'static str,
    pub http_status: u16,
}

impl Rejection {
    pub fn denied(id: Value) -> Self {
        Self {
            id,
            code: CODE_DENIED,
            message: "denied by policy",
            http_status: 403,
        }
    }

    pub fn agent_refused(id: Value) -> Self {
        Self {
            id,
            code: CODE_AGENT_REFUSED,
            message: "agent not allowed",
            http_status: 403,
        }
    }

    pub fn body(&self) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": self.id,
            "error": { "code": self.code, "message": self.message },
        })
    }
}

fn bad_request(id: Value, code: i64, message: &'static str) -> Rejection {
    Rejection {
        id,
        code,
        message,
        http_status: 400,
    }
}

pub fn parse(body: &[u8]) -> Result<Message, Rejection> {
    let parsed: Value = serde_json::from_slice(body)
        .map_err(|_| bad_request(Value::Null, CODE_PARSE_ERROR, "parse error"))?;
    let Value::Object(object) = parsed else {
        return Err(bad_request(
            Value::Null,
            CODE_INVALID_REQUEST,
            "a single JSON-RPC request is expected",
        ));
    };
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Err(bad_request(id, CODE_INVALID_REQUEST, "invalid request"));
    };
    let target = match Operation::from_method(method) {
        None => None,
        Some(operation) => {
            let params = object.get("params");
            let Some(name) = params
                .and_then(|p| p.get(operation.name_field()))
                .and_then(Value::as_str)
                .filter(|n| !n.is_empty())
            else {
                return Err(bad_request(id, CODE_INVALID_PARAMS, "invalid params"));
            };
            let arguments = params
                .and_then(|p| p.get("arguments"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            Some(Target {
                operation,
                name: name.to_owned(),
                arguments,
            })
        }
    };
    let requested_protocol = (method == "initialize")
        .then(|| {
            object
                .get("params")
                .and_then(|p| p.get("protocolVersion"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .flatten();
    Ok(Message {
        id,
        method: method.to_owned(),
        target,
        requested_protocol,
    })
}

pub fn check_headers(headers: &HeaderMap, message: &Message) -> Result<(), Rejection> {
    let mismatch = || {
        bad_request(
            message.id.clone(),
            CODE_HEADER_MISMATCH,
            "Mcp-Method or Mcp-Name does not match the request body",
        )
    };
    let header = |name: &str| headers.get(name).map(|v| v.to_str().ok());
    if let Some(method) = header(MCP_METHOD)
        && method != Some(message.method.as_str())
    {
        return Err(mismatch());
    }
    if let Some(target) = &message.target
        && let Some(name) = header(MCP_NAME)
        && name != Some(target.name.as_str())
    {
        return Err(mismatch());
    }
    Ok(())
}

pub fn with_meta(body: &[u8]) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(body).ok()?;
    let params = value
        .as_object_mut()?
        .entry("params")
        .or_insert_with(|| json!({}))
        .as_object_mut()?;
    let meta = params
        .entry("_meta")
        .or_insert_with(|| json!({}))
        .as_object_mut()?;
    let mut changed = false;
    for (key, fill) in [
        (META_PROTOCOL, json!(PROTOCOL_2026)),
        (META_CAPABILITIES, json!({})),
    ] {
        if !meta.contains_key(key) {
            meta.insert(key.to_owned(), fill);
            changed = true;
        }
    }
    changed.then(|| value.to_string().into_bytes())
}

pub fn without_meta(value: &mut Value) {
    let Some(params) = value.get_mut("params").and_then(Value::as_object_mut) else {
        return;
    };
    let Some(meta) = params.get_mut("_meta").and_then(Value::as_object_mut) else {
        return;
    };
    meta.retain(|key, _| !key.starts_with(META_PREFIX));
    if meta.is_empty() {
        params.remove("_meta");
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;

    fn headers(method: Option<&str>, name: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(method) = method {
            headers.insert(MCP_METHOD, HeaderValue::from_str(method).unwrap());
        }
        if let Some(name) = name {
            headers.insert(MCP_NAME, HeaderValue::from_str(name).unwrap());
        }
        headers
    }

    fn call(name: &str) -> Message {
        parse(
            json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":{"a":1}}})
                .to_string()
                .as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn a_tool_call_carries_its_name_and_arguments() {
        let message = call("list_deployments");
        let target = message.target.unwrap();
        assert_eq!(target.operation, Operation::CallTool);
        assert_eq!(target.name, "list_deployments");
        assert_eq!(target.arguments["a"], json!(1));
        assert_eq!(message.id, json!(7));
    }

    #[test]
    fn resources_and_prompts_are_gated_by_uri_and_name() {
        let read = parse(
            br#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"file:///a"}}"#,
        )
        .unwrap();
        let target = read.target.unwrap();
        assert_eq!(target.operation, Operation::ReadResource);
        assert_eq!(target.name, "file:///a");
        let prompt = parse(
            br#"{"jsonrpc":"2.0","id":1,"method":"prompts/get","params":{"name":"summarise"}}"#,
        )
        .unwrap();
        assert_eq!(prompt.target.unwrap().operation, Operation::GetPrompt);
    }

    #[test]
    fn other_methods_need_no_target() {
        let list = parse(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        assert!(list.target.is_none());
        assert!(list.is_tools_list());
        let note = parse(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert_eq!(note.id, Value::Null);
    }

    #[test]
    fn garbage_is_a_parse_error() {
        let rejection = parse(b"{").unwrap_err();
        assert_eq!(rejection.code, CODE_PARSE_ERROR);
        assert_eq!(rejection.http_status, 400);
    }

    #[test]
    fn a_batch_or_a_bare_response_is_refused() {
        assert_eq!(parse(b"[]").unwrap_err().code, CODE_INVALID_REQUEST);
        assert_eq!(
            parse(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#)
                .unwrap_err()
                .code,
            CODE_INVALID_REQUEST
        );
    }

    #[test]
    fn a_gated_method_without_a_name_is_invalid_params() {
        let rejection =
            parse(br#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{}}"#).unwrap_err();
        assert_eq!(rejection.code, CODE_INVALID_PARAMS);
        assert_eq!(rejection.id, json!(9));
    }

    #[test]
    fn matching_headers_pass() {
        assert!(check_headers(&headers(Some("tools/call"), Some("t")), &call("t")).is_ok());
    }

    #[test]
    fn a_different_tool_name_is_refused_with_32020() {
        let rejection =
            check_headers(&headers(Some("tools/call"), Some("other")), &call("t")).unwrap_err();
        assert_eq!(rejection.code, CODE_HEADER_MISMATCH);
        assert_eq!(rejection.http_status, 400);
        assert_eq!(rejection.id, json!(7));
    }

    #[test]
    fn a_different_method_is_refused() {
        let rejection =
            check_headers(&headers(Some("tools/list"), Some("t")), &call("t")).unwrap_err();
        assert_eq!(rejection.code, CODE_HEADER_MISMATCH);
    }

    #[test]
    fn absent_headers_are_taken_from_the_body() {
        assert!(check_headers(&headers(None, None), &call("t")).is_ok());
        assert!(check_headers(&headers(Some("tools/call"), None), &call("t")).is_ok());
    }

    #[test]
    fn a_present_header_that_disagrees_is_refused_even_when_the_other_is_absent() {
        assert!(check_headers(&headers(Some("tools/list"), None), &call("t")).is_err());
        assert!(check_headers(&headers(None, Some("other")), &call("t")).is_err());
    }

    #[test]
    fn the_client_protocol_is_kept_when_supported_else_the_newest_legacy() {
        assert_eq!(negotiate(Some("2025-03-26")), "2025-03-26");
        assert_eq!(negotiate(Some("2026-07-28")), NEWEST_LEGACY);
        assert_eq!(negotiate(Some("nonsense")), NEWEST_LEGACY);
        assert_eq!(negotiate(None), NEWEST_LEGACY);
        let init = parse(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        )
        .unwrap();
        assert!(init.is_initialize());
        assert_eq!(init.requested_protocol.as_deref(), Some("2025-06-18"));
    }

    #[test]
    fn notifications_are_recognised_by_method() {
        let note = parse(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).unwrap();
        assert!(note.is_notification());
        assert!(!call("t").is_notification());
    }

    #[test]
    fn the_2026_meta_is_added_when_missing_and_left_alone_when_present() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let added: Value = serde_json::from_slice(&with_meta(body).unwrap()).unwrap();
        assert_eq!(
            added["params"]["_meta"][META_PROTOCOL],
            json!(PROTOCOL_2026)
        );
        assert_eq!(added["params"]["_meta"][META_CAPABILITIES], json!({}));
        assert!(with_meta(added.to_string().as_bytes()).is_none());
    }

    #[test]
    fn a_partial_meta_is_completed_without_losing_the_clients_keys() {
        let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"t","_meta":{
            META_CAPABILITIES: {"sampling": {}}, "progressToken": 3}}});
        let out: Value =
            serde_json::from_slice(&with_meta(body.to_string().as_bytes()).unwrap()).unwrap();
        assert_eq!(
            out["params"]["_meta"][META_CAPABILITIES],
            json!({"sampling": {}})
        );
        assert_eq!(out["params"]["_meta"]["progressToken"], json!(3));
        assert_eq!(out["params"]["_meta"][META_PROTOCOL], json!(PROTOCOL_2026));
        assert_eq!(out["params"]["name"], json!("t"));
    }

    #[test]
    fn the_2026_meta_keys_are_dropped_and_others_kept() {
        let mut value = json!({"params":{"name":"t","_meta":{META_PROTOCOL: "x", META_CAPABILITIES: {}, "progressToken": 1}}});
        without_meta(&mut value);
        assert_eq!(value["params"]["_meta"], json!({"progressToken": 1}));
        let mut only = json!({"params":{"_meta":{META_PROTOCOL: "x"}}});
        without_meta(&mut only);
        assert_eq!(only, json!({"params":{}}));
        let mut none = json!({"method":"ping"});
        without_meta(&mut none);
        assert_eq!(none, json!({"method":"ping"}));
    }

    #[test]
    fn a_method_without_a_name_ignores_the_name_header() {
        let list = parse(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#).unwrap();
        assert!(check_headers(&headers(Some("tools/list"), None), &list).is_ok());
        assert!(check_headers(&headers(Some("tools/list"), Some("x")), &list).is_ok());
    }

    #[test]
    fn a_denial_is_a_403_json_rpc_error() {
        let rejection = Rejection::denied(json!("x"));
        assert_eq!(rejection.http_status, 403);
        assert_eq!(rejection.body()["error"]["code"], json!(CODE_DENIED));
        assert_eq!(rejection.body()["id"], json!("x"));
    }

    #[test]
    fn an_agent_refusal_is_a_403_with_its_own_code() {
        let rejection = Rejection::agent_refused(json!(3));

        assert_eq!(rejection.http_status, 403);
        assert_eq!(rejection.body()["error"]["code"], -32003);
        assert_eq!(rejection.body()["id"], 3);
    }
}
