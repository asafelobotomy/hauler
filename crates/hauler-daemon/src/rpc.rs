//! JSON-RPC 2.0 over the Unix socket, one JSON object per line.

use std::sync::Arc;

use hauler_ipc::{
    AddParams, IdParams, LimitParams, PriorityParams, RemoveParams, Request, Response, RpcError, SettingsPatch, method,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::manager::Manager;

pub async fn serve(listener: UnixListener, manager: Arc<Manager>, shutdown: CancellationToken) {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    tokio::spawn(connection(stream, manager.clone(), shutdown.clone()));
                }
                Err(e) => warn!("accept failed: {e}"),
            },
        }
    }
}

async fn connection(stream: UnixStream, manager: Arc<Manager>, shutdown: CancellationToken) {
    let (read, mut write) = stream.into_split();
    let (out, mut outbox) = mpsc::channel::<String>(256);
    let writer = tokio::spawn(async move {
        while let Some(mut line) = outbox.recv().await {
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(read).lines();
    loop {
        let line = tokio::select! {
            _ = shutdown.cancelled() => break,
            line = lines.next_line() => match line {
                Ok(Some(line)) => line,
                _ => break,
            },
        };
        if line.trim().is_empty() {
            continue;
        }
        let (reply, subscribe) = dispatch(&manager, &line).await;
        if subscribe {
            tokio::spawn(forward_events(manager.subscribe(), out.clone()));
        }
        if let Some(reply) = reply
            && out.send(reply).await.is_err()
        {
            break;
        }
    }
    drop(out);
    let _ = writer.await;
}

async fn forward_events(mut events: broadcast::Receiver<hauler_ipc::Event>, out: mpsc::Sender<String>) {
    loop {
        let event = match events.recv().await {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(n)) => {
                debug!("subscriber lagged by {n} events");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let note = Request {
            jsonrpc: "2.0".into(),
            id: None,
            method: method::EVENT.into(),
            params: serde_json::to_value(event).unwrap(),
        };
        if out.send(serde_json::to_string(&note).unwrap()).await.is_err() {
            return;
        }
    }
}

/// Handle one request line. Returns the reply (none for notifications) and
/// whether the client asked for events.
async fn dispatch(manager: &Manager, line: &str) -> (Option<String>, bool) {
    let req: Request = match serde_json::from_str(line) {
        Ok(r) => r,
        Err(e) => return (Some(reply(Value::Null, Err(RpcError::new(RpcError::PARSE, e.to_string())))), false),
    };
    let id = req.id.clone();
    if req.jsonrpc != "2.0" {
        let err = RpcError::new(RpcError::INVALID_REQUEST, "jsonrpc must be \"2.0\"");
        return (id.map(|id| reply(id, Err(err))), false);
    }
    let subscribe = req.method == method::SUBSCRIBE && id.is_some();
    let result = call(manager, &req.method, req.params).await;
    (id.map(|id| reply(id, result)), subscribe)
}

async fn call(m: &Manager, method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
        method::ADD => ok(m.add(parse::<AddParams>(params)?)),
        method::LIST => ok(Ok(m.list())),
        method::GET => ok(m.get(parse::<IdParams>(params)?.id)),
        method::PAUSE => ok(m.pause(parse::<IdParams>(params)?.id)),
        method::RESUME => ok(m.resume(parse::<IdParams>(params)?.id)),
        method::REMOVE => {
            let p: RemoveParams = parse(params)?;
            ok(m.remove(p.id, p.delete_files).await)
        }
        method::SET_LIMIT => {
            let p: LimitParams = parse(params)?;
            ok(m.set_limit(p.id, p.bytes_per_sec))
        }
        method::SET_PRIORITY => {
            let p: PriorityParams = parse(params)?;
            ok(m.set_priority(p.id, p.priority))
        }
        method::PAUSE_ALL => ok(Ok(m.pause_all())),
        method::RESUME_ALL => ok(Ok(m.resume_all())),
        method::GET_SETTINGS => ok(Ok(m.settings())),
        method::SET_SETTINGS => ok(m.set_settings(parse::<SettingsPatch>(params)?)),
        method::STATUS => ok(Ok(m.status())),
        method::SEGMENTS => ok(m.segments(parse::<IdParams>(params)?.id)),
        method::SUBSCRIBE => Ok(Value::Bool(true)),
        other => Err(RpcError::new(RpcError::METHOD_NOT_FOUND, format!("unknown method {other}"))),
    }
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params).map_err(|e| RpcError::new(RpcError::INVALID_PARAMS, e.to_string()))
}

fn ok<T: Serialize>(r: Result<T, RpcError>) -> Result<Value, RpcError> {
    r.map(|v| serde_json::to_value(v).unwrap())
}

fn reply(id: Value, result: Result<Value, RpcError>) -> String {
    let (result, error) = match result {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e)),
    };
    serde_json::to_string(&Response { jsonrpc: "2.0".into(), id, result, error }).unwrap()
}
