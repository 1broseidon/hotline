//! A paired viewer operates on its computer's home, never on desk-host paths.
//! HTTP credentials stay here; each socket owns its bounded queues and spools.
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncSeekExt, AsyncWriteExt},
    sync::mpsc,
};

const CHUNK: usize = 512 * 1024;
pub(super) const MAX_REQUEST: usize = CHUNK.div_ceil(3) * 4 + 4096;
const JSON_LIMIT: usize = 4 * 1024 * 1024;
const CREATE_ONLY: &str = "x-hotline-upload-create-only";

pub(super) struct Relay {
    requests: mpsc::Sender<Value>,
    pub replies: mpsc::Receiver<Value>,
    worker: tokio::task::JoinHandle<()>,
}
impl Drop for Relay {
    fn drop(&mut self) {
        self.worker.abort();
    }
}
impl Relay {
    pub fn start(port: u16, token: String, root: PathBuf) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| "The computer's file relay could not start.")?;
        let (requests, mut incoming) = mpsc::channel::<Value>(4);
        let (outgoing, replies) = mpsc::channel(4);
        let worker = tokio::spawn(async move {
            let mut state = Files {
                http,
                base: format!("http://127.0.0.1:{port}"),
                token,
                root,
                uploads: HashMap::new(),
                downloads: HashMap::new(),
            };
            while let Some(value) = incoming.recv().await {
                let id = value.get("id").cloned().unwrap_or(Value::Null);
                let result = match serde_json::from_value::<Request>(value) {
                    Ok(request)
                        if request.kind == "files"
                            && (id.is_string() || id.is_i64() || id.is_u64()) =>
                    {
                        match tokio::time::timeout(
                            Duration::from_secs(25),
                            state.handle(request.op),
                        )
                        .await
                        {
                            Ok(result) => result,
                            Err(_) => {
                                // A cancelled write may have advanced its file. Do not
                                // let a retry reuse an offset we can no longer confirm.
                                state.uploads.clear();
                                state.downloads.clear();
                                Err("The computer's file request timed out. Restart unfinished transfers and check the destination before retrying an upload.".into())
                            }
                        }
                    }
                    _ => Err("Invalid computer file request.".into()),
                };
                if outgoing.send(reply(id, result)).await.is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            requests,
            replies,
            worker,
        })
    }
    /// Never let file IO or a burst stop the screen/control forwarding loop.
    pub fn enqueue(&self, request: Value) -> Option<Value> {
        self.requests.try_send(request).err().map(|error| {
            let request = error.into_inner();
            reply(
                request.get("id").cloned().unwrap_or(Value::Null),
                Err("The computer's file queue is busy. Try again shortly.".into()),
            )
        })
    }
}
fn reply(id: Value, result: Result<Value, String>) -> Value {
    match result {
        Ok(result) => json!({"type":"files","id":id,"ok":true,"result":result}),
        Err(error) => json!({"type":"files","id":id,"ok":false,"error":error}),
    }
}
#[derive(Deserialize)]
struct Request {
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    op: Op,
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", rename_all_fields = "camelCase")]
enum Op {
    List {
        #[serde(default)]
        path: String,
    },
    Download {
        path: String,
        offset: u64,
    },
    UploadStart {
        path: String,
    },
    UploadChunk {
        upload_id: String,
        offset: u64,
        data: String,
    },
    UploadFinish {
        upload_id: String,
    },
    UploadCancel {
        upload_id: String,
    },
}
#[derive(Deserialize, Serialize)]
struct Listing {
    path: String,
    home: String,
    entries: Vec<Entry>,
}
#[derive(Deserialize, Serialize)]
struct Entry {
    name: String,
    is_dir: bool,
    size: u64,
}
struct Upload {
    file: tokio::fs::File,
    _temporary: tempfile::NamedTempFile,
    path: String,
    offset: u64,
}
struct Download {
    response: reqwest::Response,
    pending: Bytes,
    size: u64,
    offset: u64,
}
impl Download {
    async fn take(&mut self, count: usize) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(count);
        while bytes.len() < count {
            if self.pending.is_empty() {
                self.pending = self
                    .response
                    .chunk()
                    .await
                    .map_err(|_| "The computer's download stopped.")?
                    .ok_or("The file changed while downloading. Start again.")?;
                if self.pending.is_empty() {
                    continue;
                }
            }
            let size = (count - bytes.len()).min(self.pending.len());
            bytes.extend_from_slice(&self.pending.split_to(size));
            self.offset += size as u64;
        }
        Ok(bytes)
    }
}
struct Files {
    http: reqwest::Client,
    base: String,
    token: String,
    root: PathBuf,
    uploads: HashMap<String, Upload>,
    downloads: HashMap<String, Download>,
}
impl Files {
    fn request(&self, method: reqwest::Method, route: &str, path: &str) -> reqwest::RequestBuilder {
        let mut url = url::Url::parse(&format!("{}{route}", self.base)).expect("loopback file URL");
        url.query_pairs_mut().append_pair("path", path);
        if method == reqwest::Method::POST {
            url.query_pairs_mut().append_pair("create_only", "true");
        }
        self.http.request(method, url).bearer_auth(&self.token)
    }
    async fn listing(&self, path: &str) -> Result<(Listing, bool), String> {
        let path = if path.is_empty() {
            crate::computer::HOME_MOUNT
        } else {
            path
        };
        let response = self
            .request(reqwest::Method::GET, "/files", path)
            .send()
            .await
            .map_err(|_| "The computer's file service did not answer.")?;
        let create_only = response
            .headers()
            .get(CREATE_ONLY)
            .is_some_and(|v| v == "1");
        let listing = serde_json::from_value(json_response(response).await?)
            .map_err(|_| "The computer sent an invalid file listing.")?;
        Ok((listing, create_only))
    }
    async fn destination(&self, path: &str) -> Result<String, String> {
        let (parent, name) = path
            .rsplit_once('/')
            .filter(|(_, name)| !name.is_empty())
            .ok_or("Choose an absolute destination filename on the computer.")?;
        if !path.starts_with('/')
            || matches!(name, "." | "..")
            || path.contains('\0')
            || path.len() > 4096
        {
            return Err("Choose an absolute destination filename on the computer.".into());
        }
        let (listing, create_only) = self
            .listing(if parent.is_empty() { "/" } else { parent })
            .await?;
        if !create_only {
            // Older POST /files silently overwrites. A preflight alone cannot
            // protect a concurrent writer; never pretend it is create-only.
            return Err(
                "Update this computer to support uploads without overwriting existing files."
                    .into(),
            );
        }
        if listing.entries.iter().any(|entry| entry.name == name) {
            return Err("That filename already exists. Choose a new name.".into());
        }
        Ok(format!("{}/{name}", listing.path.trim_end_matches('/')))
    }
    async fn handle(&mut self, op: Op) -> Result<Value, String> {
        match op {
            Op::List { path } => serde_json::to_value(self.listing(&path).await?.0)
                .map_err(|_| "Invalid file listing.".into()),
            Op::Download { path, offset } => self.download(path, offset).await,
            Op::UploadStart { path } => {
                if self.uploads.len() >= 8 {
                    return Err("Finish or cancel an upload before starting another.".into());
                }
                let path = self.destination(&path).await?;
                tokio::fs::create_dir_all(&self.root)
                    .await
                    .map_err(|_| "Could not stage the upload on the desk.")?;
                let temporary = tempfile::NamedTempFile::new_in(&self.root)
                    .map_err(|_| "Could not stage the upload on the desk.")?;
                let file = tokio::fs::File::from_std(
                    temporary
                        .reopen()
                        .map_err(|_| "Could not stage the upload on the desk.")?,
                );
                let id = uuid::Uuid::new_v4().to_string();
                self.uploads.insert(
                    id.clone(),
                    Upload {
                        file,
                        _temporary: temporary,
                        path,
                        offset: 0,
                    },
                );
                Ok(json!({"uploadId":id,"offset":0}))
            }
            Op::UploadChunk {
                upload_id,
                offset,
                data,
            } => {
                let upload = self
                    .uploads
                    .get_mut(&upload_id)
                    .ok_or("That upload belongs to another connection or has ended.")?;
                if offset != upload.offset {
                    return Err("The upload offset does not match.".into());
                }
                if data.len() > CHUNK.div_ceil(3) * 4 {
                    return Err("Send at most 512 KiB per upload chunk.".into());
                }
                let bytes = STANDARD.decode(data).map_err(|_| "Invalid upload data.")?;
                if bytes.len() > CHUNK {
                    return Err("Send at most 512 KiB per upload chunk.".into());
                }
                if upload.file.write_all(&bytes).await.is_err() {
                    self.uploads.remove(&upload_id);
                    return Err("Could not stage the upload on the desk.".into());
                }
                upload.offset += bytes.len() as u64;
                Ok(json!({"offset":upload.offset}))
            }
            Op::UploadFinish { upload_id } => {
                let mut upload = self
                    .uploads
                    .remove(&upload_id)
                    .ok_or("That upload belongs to another connection or has ended.")?;
                self.destination(&upload.path).await?;
                upload
                    .file
                    .flush()
                    .await
                    .map_err(|_| "Could not finish staging the upload.")?;
                upload
                    .file
                    .rewind()
                    .await
                    .map_err(|_| "Could not read the staged upload.")?;
                let response = self.request(reqwest::Method::POST, "/files", &upload.path)
                    .header(reqwest::header::CONTENT_LENGTH, upload.offset)
                    .body(reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::with_capacity(upload.file, CHUNK)))
                    .send().await.map_err(|_| "Could not confirm the upload. Check its destination before trying again.")?;
                let result = json_response(response).await?;
                if result["path"] != upload.path || result["bytes"] != upload.offset {
                    return Err("The computer did not confirm the complete upload. Check its destination before trying again.".into());
                }
                Ok(json!({"path":upload.path,"size":upload.offset}))
            }
            Op::UploadCancel { upload_id } => {
                self.uploads
                    .remove(&upload_id)
                    .ok_or("That upload belongs to another connection or has ended.")?;
                Ok(Value::Null)
            }
        }
    }
    async fn download(&mut self, path: String, offset: u64) -> Result<Value, String> {
        let cached = self
            .downloads
            .remove(&path)
            .filter(|download| download.offset == offset);
        let mut download = if let Some(cached) = cached {
            cached
        } else {
            if self.downloads.len() >= 4 {
                return Err("Finish another download before starting this one.".into());
            }
            let response = self
                .request(reqwest::Method::GET, "/files/download", &path)
                .send()
                .await
                .map_err(|_| "The computer's download did not answer.")?;
            status(&response)?;
            let size = response
                .content_length()
                .ok_or("The computer did not report the file size.")?;
            if offset > size {
                return Err("The download offset is past the end of the file.".into());
            }
            let mut download = Download {
                response,
                pending: Bytes::new(),
                size,
                offset: 0,
            };
            // Old computer releases ignore Range. Skip once, then retain the
            // response across chunks rather than downloading the prefix again.
            while download.offset < offset {
                let count = (offset - download.offset).min(CHUNK as u64) as usize;
                download.take(count).await?;
            }
            download
        };
        let count = (download.size - download.offset).min(CHUNK as u64) as usize;
        let bytes = download.take(count).await?;
        let next = (download.offset < download.size).then_some(download.offset);
        let result = json!({"name":path.rsplit('/').next().unwrap_or("file"),"size":download.size,"offset":offset,"data":STANDARD.encode(bytes),"next":next});
        if next.is_some() {
            self.downloads.insert(path, download);
        }
        Ok(result)
    }
}
fn status(response: &reqwest::Response) -> Result<(), String> {
    if response.status().is_success() {
        Ok(())
    } else if response.status() == reqwest::StatusCode::CONFLICT {
        Err("That filename already exists. Choose a new name.".into())
    } else {
        // Do not reflect an upstream error page or credentials to the viewer.
        Err(format!(
            "The computer refused the file request ({}).",
            response.status()
        ))
    }
}
async fn json_response(mut response: reqwest::Response) -> Result<Value, String> {
    status(&response)?;
    if response
        .content_length()
        .is_some_and(|size| size > JSON_LIMIT as u64)
    {
        return Err("The computer's file response is too large.".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The computer's file response stopped.")?
    {
        if bytes.len() + chunk.len() > JSON_LIMIT {
            return Err("The computer's file response is too large.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "The computer sent an invalid file response.".into())
}
