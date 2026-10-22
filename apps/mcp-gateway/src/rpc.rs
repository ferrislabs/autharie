use http::HeaderMap;
use serde_json::{Map, Value, json};

pub const CODE_PARSE_ERROR: i64 = -32700;
pub const CODE_INVALID_REQUEST: i64 = -32600;
pub const CODE_INVALID_PARAMS: i64 = -32602;
pub const CODE_DENIED: i64 = -32001;
pub const CODE_HEADER_MISMATCH: i64 = -32020;

pub const MCP_METHOD: &str = "mcp-method";
pub const MCP_NAME: &str = "mcp-name";

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
}

impl Message {
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
    Ok(Message {
        id,
        method: method.to_owned(),
        target,
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
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if header(MCP_METHOD) != Some(message.method.as_str()) {
        return Err(mismatch());
    }
    if let Some(target) = &message.target
        && header(MCP_NAME) != Some(target.name.as_str())
    {
        return Err(mismatch());
    }
    Ok(())
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
    fn missing_headers_are_refused() {
        assert!(check_headers(&headers(None, None), &call("t")).is_err());
        assert!(check_headers(&headers(Some("tools/call"), None), &call("t")).is_err());
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
}
