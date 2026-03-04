use serde::{Deserialize, Serialize};
use serde_json::Value;

use grid_core::{
    KvContainsRequest, KvDeleteRequest, KvGetRequest, KvPrefixScanRequest, KvPutRequest,
    SectorAppendRequest, SectorBatchAppendRequest, SectorBatchLogLengthRequest,
    SectorLogLengthRequest, SectorReadLogRequest, SectorRequest, SectorResponse,
};

use crate::{NodeStatus, SectorDispatch};

const PARSE_ERROR: i32 = -32700;
const INVALID_REQUEST: i32 = -32600;
const METHOD_NOT_FOUND: i32 = -32601;
const INVALID_PARAMS: i32 = -32602;
const INTERNAL_ERROR: i32 = -32603;

#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub(crate) fn parse_error(msg: &str) -> JsonRpcResponse {
    error_response(Value::Null, PARSE_ERROR, msg)
}

pub(crate) fn dispatch(
    handler: &dyn SectorDispatch,
    node_status: Option<&dyn NodeStatus>,
    req: &JsonRpcRequest,
) -> JsonRpcResponse {
    if req.jsonrpc != "2.0" {
        return error_response(req.id.clone(), INVALID_REQUEST, "Invalid JSON-RPC version");
    }

    match req.method.as_str() {
        "node.status" => match node_status {
            Some(ns) => success_response(req.id.clone(), ns.status()),
            None => error_response(req.id.clone(), INTERNAL_ERROR, "node status not available"),
        },
        "sector.append" => {
            dispatch_typed::<SectorAppendRequest>(handler, req, SectorRequest::Append)
        }
        "sector.readLog" => {
            dispatch_typed::<SectorReadLogRequest>(handler, req, SectorRequest::ReadLog)
        }
        "sector.logLength" => {
            dispatch_typed::<SectorLogLengthRequest>(handler, req, SectorRequest::LogLength)
        }
        "sector.batchAppend" => {
            dispatch_typed::<SectorBatchAppendRequest>(handler, req, SectorRequest::BatchAppend)
        }
        "sector.batchLogLength" => dispatch_typed::<SectorBatchLogLengthRequest>(
            handler,
            req,
            SectorRequest::BatchLogLength,
        ),
        "kv.get" => dispatch_typed::<KvGetRequest>(handler, req, SectorRequest::KvGet),
        "kv.put" => dispatch_typed::<KvPutRequest>(handler, req, SectorRequest::KvPut),
        "kv.delete" => dispatch_typed::<KvDeleteRequest>(handler, req, SectorRequest::KvDelete),
        "kv.contains" => {
            dispatch_typed::<KvContainsRequest>(handler, req, SectorRequest::KvContains)
        }
        "kv.prefixScan" => {
            dispatch_typed::<KvPrefixScanRequest>(handler, req, SectorRequest::KvPrefixScan)
        }
        _ => error_response(
            req.id.clone(),
            METHOD_NOT_FOUND,
            &format!("Unknown method: {}", req.method),
        ),
    }
}

fn dispatch_typed<P>(
    handler: &dyn SectorDispatch,
    req: &JsonRpcRequest,
    wrap: fn(P) -> SectorRequest,
) -> JsonRpcResponse
where
    P: serde::de::DeserializeOwned,
{
    let params: P = match serde_json::from_value(req.params.clone()) {
        Ok(p) => p,
        Err(e) => return error_response(req.id.clone(), INVALID_PARAMS, &e.to_string()),
    };
    let sector_req = wrap(params);
    let sector_resp = handler.dispatch(&sector_req);
    response_from_sector(req.id.clone(), &sector_resp)
}

fn response_from_sector(id: Value, resp: &SectorResponse) -> JsonRpcResponse {
    match serde_json::to_value(resp) {
        Ok(v) => success_response(id, v),
        Err(e) => error_response(id, INTERNAL_ERROR, &e.to_string()),
    }
}

fn success_response(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn error_response(id: Value, code: i32, message: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.to_string(),
            data: None,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grid_core::{
        KvContainsResponse, KvDeleteResponse, KvGetResponse, KvPrefixScanResponse, KvPutResponse,
        SectorLogLengthResponse, SectorRequest, SectorResponse,
    };

    struct MockHandler;

    impl SectorDispatch for MockHandler {
        fn dispatch(&self, req: &SectorRequest) -> SectorResponse {
            match req {
                SectorRequest::LogLength(_) => SectorResponse::LogLength(SectorLogLengthResponse {
                    length: 42,
                    error_code: None,
                }),
                SectorRequest::KvGet(_) => SectorResponse::KvGet(KvGetResponse {
                    value: Some(b"hello".to_vec()),
                    error_code: None,
                }),
                SectorRequest::KvPut(_) => SectorResponse::KvPut(KvPutResponse {
                    ok: true,
                    error_code: None,
                }),
                SectorRequest::KvDelete(_) => SectorResponse::KvDelete(KvDeleteResponse {
                    ok: true,
                    error_code: None,
                }),
                SectorRequest::KvContains(_) => SectorResponse::KvContains(KvContainsResponse {
                    exists: true,
                    error_code: None,
                }),
                SectorRequest::KvPrefixScan(_) => {
                    SectorResponse::KvPrefixScan(KvPrefixScanResponse {
                        entries: Vec::new(),
                        error_code: None,
                    })
                }
                _ => unimplemented!("MockHandler: unsupported request"),
            }
        }
    }

    fn make_request(jsonrpc: &str, method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: jsonrpc.to_string(),
            id: Value::Number(1.into()),
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn parse_error_on_invalid_json() {
        let resp = parse_error("unexpected token");
        assert_eq!(resp.id, Value::Null);
        assert!(resp.result.is_none());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, PARSE_ERROR);
        assert!(err.message.contains("unexpected token"));
    }

    #[test]
    fn invalid_jsonrpc_version_returns_error() {
        let req = make_request("1.0", "sector.logLength", Value::Null);
        let resp = dispatch(&MockHandler, None, &req);
        assert!(resp.result.is_none());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, INVALID_REQUEST);
        assert!(err.message.contains("version"));
    }

    #[test]
    fn unknown_method_returns_method_not_found() {
        let req = make_request("2.0", "sector.doesNotExist", Value::Null);
        let resp = dispatch(&MockHandler, None, &req);
        assert!(resp.result.is_none());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, METHOD_NOT_FOUND);
        assert!(err.message.contains("sector.doesNotExist"));
    }

    #[test]
    fn log_length_dispatch_returns_result() {
        let program_id_bytes: Vec<u8> = vec![1; 32];
        let params = serde_json::json!({
            "program_id": program_id_bytes,
            "sector_id": [2, 3, 4],
        });
        let req = make_request("2.0", "sector.logLength", params);
        let resp = dispatch(&MockHandler, None, &req);

        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        let result = resp.result.as_ref().unwrap();
        assert_eq!(result["LogLength"]["length"], 42);
    }

    #[test]
    fn invalid_params_returns_error() {
        let params = serde_json::json!({"wrong_field": true});
        let req = make_request("2.0", "sector.logLength", params);
        let resp = dispatch(&MockHandler, None, &req);

        assert!(resp.result.is_none());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, INVALID_PARAMS);
    }

    fn kv_params(key: &[u8]) -> Value {
        let program_id_bytes: Vec<u8> = vec![1; 32];
        serde_json::json!({
            "program_id": program_id_bytes,
            "key": key,
        })
    }

    #[test]
    fn kv_get_dispatch_returns_result() {
        let req = make_request("2.0", "kv.get", kv_params(b"mykey"));
        let resp = dispatch(&MockHandler, None, &req);
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        let result = resp.result.as_ref().unwrap();
        assert!(result["KvGet"]["value"].is_array());
    }

    #[test]
    fn kv_put_dispatch_returns_result() {
        let mut params = kv_params(b"mykey");
        params["value"] = serde_json::json!(vec![1u8, 2, 3]);
        let req = make_request("2.0", "kv.put", params);
        let resp = dispatch(&MockHandler, None, &req);
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        let result = resp.result.as_ref().unwrap();
        assert_eq!(result["KvPut"]["ok"], true);
    }

    #[test]
    fn kv_delete_dispatch_returns_result() {
        let req = make_request("2.0", "kv.delete", kv_params(b"mykey"));
        let resp = dispatch(&MockHandler, None, &req);
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        let result = resp.result.as_ref().unwrap();
        assert_eq!(result["KvDelete"]["ok"], true);
    }

    #[test]
    fn kv_contains_dispatch_returns_result() {
        let req = make_request("2.0", "kv.contains", kv_params(b"mykey"));
        let resp = dispatch(&MockHandler, None, &req);
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        let result = resp.result.as_ref().unwrap();
        assert_eq!(result["KvContains"]["exists"], true);
    }

    struct MockNodeStatus;

    impl NodeStatus for MockNodeStatus {
        fn status(&self) -> Value {
            serde_json::json!({"zode_id": "Zx_test", "peer_count": 3})
        }
    }

    #[test]
    fn node_status_returns_result() {
        let req = make_request("2.0", "node.status", Value::Null);
        let ns = MockNodeStatus;
        let resp = dispatch(&MockHandler, Some(&ns), &req);
        assert!(resp.error.is_none(), "unexpected error: {:?}", resp.error);
        let result = resp.result.as_ref().unwrap();
        assert_eq!(result["zode_id"], "Zx_test");
        assert_eq!(result["peer_count"], 3);
    }

    #[test]
    fn node_status_without_provider_returns_error() {
        let req = make_request("2.0", "node.status", Value::Null);
        let resp = dispatch(&MockHandler, None, &req);
        assert!(resp.result.is_none());
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, INTERNAL_ERROR);
    }
}
