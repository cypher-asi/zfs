use serde::{Deserialize, Serialize};
use serde_json::Value;

use grid_core::{
    SectorAppendRequest, SectorBatchAppendRequest, SectorBatchLogLengthRequest,
    SectorLogLengthRequest, SectorReadLogRequest, SectorRequest, SectorResponse,
};

use crate::SectorDispatch;

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

pub(crate) fn dispatch(handler: &dyn SectorDispatch, req: &JsonRpcRequest) -> JsonRpcResponse {
    if req.jsonrpc != "2.0" {
        return error_response(req.id.clone(), INVALID_REQUEST, "Invalid JSON-RPC version");
    }

    match req.method.as_str() {
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
