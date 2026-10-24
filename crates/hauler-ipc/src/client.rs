//! The client side: one request at a time, or a stream of events.

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::{
    AddParams, Event, IdParams, LimitParams, PriorityParams, RemoveParams, Request, Response, RpcError, SegmentMap,
    Settings, SettingsPatch, Status, TaskId, TaskInfo, check_private_dir, default_socket_path, method,
};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("cannot reach haulerd at {path}: {source}")]
    Connect { path: PathBuf, source: std::io::Error },
    #[error("connection to haulerd failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad message from haulerd: {0}")]
    Json(#[from] serde_json::Error),
    #[error("haulerd closed the connection")]
    Closed,
    #[error(transparent)]
    Rpc(#[from] RpcError),
}

/// One request at a time over one connection.
pub struct Client {
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
    next_id: u64,
}

impl Client {
    pub async fn connect(path: impl Into<PathBuf>) -> Result<Self, ClientError> {
        let path = path.into();
        if let Some(dir) = path.parent()
            && let Err(source) = check_private_dir(dir)
        {
            return Err(ClientError::Connect { path, source });
        }
        let stream = UnixStream::connect(&path).await.map_err(|source| ClientError::Connect { path, source })?;
        let (read, writer) = stream.into_split();
        Ok(Self { lines: BufReader::new(read).lines(), writer, next_id: 1 })
    }

    pub async fn connect_default() -> Result<Self, ClientError> {
        Self::connect(default_socket_path()).await
    }

    pub async fn call<P: Serialize, R: DeserializeOwned>(&mut self, method: &str, params: P) -> Result<R, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = Request {
            jsonrpc: "2.0".into(),
            id: Some(id.into()),
            method: method.into(),
            params: serde_json::to_value(params)?,
        };
        let mut line = serde_json::to_vec(&req)?;
        line.push(b'\n');
        self.writer.write_all(&line).await?;
        loop {
            let line = self.lines.next_line().await?.ok_or(ClientError::Closed)?;
            let value: Value = serde_json::from_str(&line)?;
            if value.get("id") != Some(&Value::from(id)) {
                continue; // a notification
            }
            let resp: Response = serde_json::from_value(value)?;
            if let Some(err) = resp.error {
                return Err(err.into());
            }
            return Ok(serde_json::from_value(resp.result.unwrap_or(Value::Null))?);
        }
    }

    /// Turn this connection into an event stream.
    pub async fn subscribe(mut self) -> Result<Events, ClientError> {
        self.call::<_, Value>(method::SUBSCRIBE, Value::Null).await?;
        Ok(Events { lines: self.lines, _writer: self.writer })
    }

    pub async fn add(&mut self, params: &AddParams) -> Result<TaskInfo, ClientError> {
        self.call(method::ADD, params).await
    }

    pub async fn list(&mut self) -> Result<Vec<TaskInfo>, ClientError> {
        self.call(method::LIST, Value::Null).await
    }

    pub async fn get(&mut self, id: TaskId) -> Result<TaskInfo, ClientError> {
        self.call(method::GET, IdParams { id }).await
    }

    pub async fn pause(&mut self, id: TaskId) -> Result<TaskInfo, ClientError> {
        self.call(method::PAUSE, IdParams { id }).await
    }

    pub async fn resume(&mut self, id: TaskId) -> Result<TaskInfo, ClientError> {
        self.call(method::RESUME, IdParams { id }).await
    }

    pub async fn remove(&mut self, id: TaskId, delete_files: bool) -> Result<(), ClientError> {
        self.call(method::REMOVE, RemoveParams { id, delete_files }).await
    }

    pub async fn set_limit(&mut self, id: TaskId, bytes_per_sec: u64) -> Result<TaskInfo, ClientError> {
        self.call(method::SET_LIMIT, LimitParams { id, bytes_per_sec }).await
    }

    pub async fn set_priority(&mut self, id: TaskId, priority: i32) -> Result<TaskInfo, ClientError> {
        self.call(method::SET_PRIORITY, PriorityParams { id, priority }).await
    }

    pub async fn pause_all(&mut self) -> Result<u32, ClientError> {
        self.call(method::PAUSE_ALL, Value::Null).await
    }

    pub async fn resume_all(&mut self) -> Result<u32, ClientError> {
        self.call(method::RESUME_ALL, Value::Null).await
    }

    pub async fn settings(&mut self) -> Result<Settings, ClientError> {
        self.call(method::GET_SETTINGS, Value::Null).await
    }

    pub async fn set_settings(&mut self, patch: &SettingsPatch) -> Result<Settings, ClientError> {
        self.call(method::SET_SETTINGS, patch).await
    }

    pub async fn status(&mut self) -> Result<Status, ClientError> {
        self.call(method::STATUS, Value::Null).await
    }

    pub async fn segments(&mut self, id: TaskId) -> Result<SegmentMap, ClientError> {
        self.call(method::SEGMENTS, IdParams { id }).await
    }
}

pub struct Events {
    lines: Lines<BufReader<OwnedReadHalf>>,
    _writer: OwnedWriteHalf,
}

impl Events {
    /// The next event, or `None` when the daemon goes away.
    pub async fn next(&mut self) -> Result<Option<Event>, ClientError> {
        loop {
            let Some(line) = self.lines.next_line().await? else { return Ok(None) };
            let req: Request = serde_json::from_str(&line)?;
            if req.method == method::EVENT {
                return Ok(Some(serde_json::from_value(req.params)?));
            }
        }
    }
}
