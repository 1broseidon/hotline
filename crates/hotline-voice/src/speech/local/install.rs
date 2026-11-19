//! Downloading the desk's speech models, which the owner asks for one at a
//! time: nothing is installed until they do.
//!
//! Every model is a few files from Hugging Face, each fetched at one commit
//! and pinned by its size and SHA-256. A commit never changes what it holds,
//! so a model re-uploaded upstream neither breaks its download nor changes
//! what the desk installs; an address that can be re-uploaded (a release's
//! file, a branch) is not used. Each file is hashed as it arrives and refused
//! whole if it does not match, and is kept under the desk's own name for it,
//! so nothing downloaded chooses a path. The model appears only when its
//! directory is complete, by renaming it into place, so a crash or a cancel
//! leaves no half model to load.

use super::{Engine, FileHash, Installed, MANIFEST, Manifest, installed, models_dir, unload};
use futures_util::StreamExt;
use hotline_contract::{SpeechModel, SpeechModelState};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

/// A model the desk can download.
#[derive(Clone, Debug)]
pub struct Model {
    pub id: String,
    pub name: String,
    /// A word or two after the name in Settings, to choose by.
    pub tag: Option<String>,
    pub detail: String,
    pub engine: Engine,
    /// Every file of the model, each pinned.
    pub files: Vec<Pinned>,
    pub credit: String,
    pub licence_url: String,
    /// A model offered here that hears more languages, by id, for Settings
    /// to suggest to someone whose window is not in English while this is
    /// the only one installed.
    pub more_languages: Option<String>,
}

/// One file of a model, as it must arrive.
#[derive(Clone, Debug)]
pub struct Pinned {
    /// What the desk calls it on disk: one of its engine's files.
    pub name: String,
    pub url: String,
    pub bytes: u64,
    /// Lowercase hex of its SHA-256.
    pub sha256: String,
}

impl Model {
    /// What its download comes to, and so what it takes on disk: its files
    /// are kept as they arrive.
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|file| file.bytes).sum()
    }
}

/// One file of a model: the desk's `name` for it, where it is fetched from,
/// and what it must be.
pub(super) fn pinned(name: &str, url: String, bytes: u64, sha256: &str) -> Pinned {
    Pinned {
        name: name.into(),
        url,
        bytes,
        sha256: sha256.into(),
    }
}

const CC_BY: &str = "https://creativecommons.org/licenses/by/4.0/";

/// sherpa-onnx's int8 export of Parakeet TDT 0.6B v3 at its commit of
/// 2025-08-16: the files Hotline first pinned, in the archive sherpa-onnx's
/// `asr-models` release had then. On 2026-11-07 sherpa-onnx exported the
/// model again from the same checkpoint to add a model card (#4167), and
/// replaced that archive at the same address with different bytes, which a
/// pin on the address could only refuse.
const PARAKEET: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/2bda32ec70b097a55adaa07d9a7173915b43cc78";
/// sherpa-onnx's Whisper large-v3-turbo at its one upload, 2024-10-02: the
/// same bytes as its archive in the `asr-models` release.
const WHISPER: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-whisper-turbo/resolve/2ca6ff69fc878651b770880507669577ac41c2ff";
/// sherpa-onnx's own repository for Parakeet TDT 110M int8 holds no files,
/// so this is a mirror of 2026-08-10 whose files are byte for byte those of
/// sherpa-onnx's archive in the `asr-models` release (uploaded 2026-05-03),
/// which Hotline pinned before: the hashes, not the host, say what it is.
const PARAKEET_ENGLISH: &str = "https://huggingface.co/punitd/sherpa-onnx-nemo-parakeet_tdt_transducer_110m-en-36000-int8/resolve/66a4fa70643dc7ce25c9b38b2f87e1b35ddad33d";
/// sherpa-onnx's Moonshine Base int8 at its commit of 2024-10-28: the same
/// bytes as its archive in the `asr-models` release.
const MOONSHINE: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-moonshine-base-en-int8/resolve/052b0798ad1bf046a140fdd4efcd9426530fa3f5";

/// The models Hotline offers, in the order automatic hearing takes the first
/// one installed: the multilingual Parakeet, Whisper, which hears more
/// languages but takes ten times as long, then the English-only ones. For
/// English, Parakeet English heard as well as Parakeet in people's own use,
/// in a quarter of the time. Every one runs on the engine the desk links
/// (sherpa-onnx 1.13.8); Moonshine is its first base model, because the 2026
/// one fails in this engine on anything longer than a few seconds. The
/// desk's voice comes last, where this build can run it ([`super::voice`]);
/// it never hears, so its place does not change what hears.
pub fn catalogue() -> Vec<Model> {
    let mut models = vec![
        Model {
            id: "parakeet-tdt-0.6b-v3".into(),
            name: "Parakeet".into(),
            tag: Some("25 languages".into()),
            detail: "25 European languages".into(),
            engine: Engine::Transducer,
            files: vec![
                pinned(
                    "encoder.int8.onnx",
                    format!("{PARAKEET}/encoder.int8.onnx"),
                    652_184_281,
                    "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
                ),
                pinned(
                    "decoder.int8.onnx",
                    format!("{PARAKEET}/decoder.int8.onnx"),
                    11_845_275,
                    "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
                ),
                pinned(
                    "joiner.int8.onnx",
                    format!("{PARAKEET}/joiner.int8.onnx"),
                    6_355_277,
                    "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
                ),
                pinned(
                    "tokens.txt",
                    format!("{PARAKEET}/tokens.txt"),
                    93_939,
                    "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                ),
            ],
            credit: "NVIDIA Parakeet TDT 0.6B v3, CC BY 4.0, quantized by sherpa-onnx".into(),
            licence_url: CC_BY.into(),
            more_languages: None,
        },
        Model {
            id: "whisper-large-v3-turbo".into(),
            name: "Whisper".into(),
            tag: Some("99 languages".into()),
            detail: "99 languages, slower".into(),
            engine: Engine::Whisper,
            files: vec![
                pinned(
                    "encoder.int8.onnx",
                    format!("{WHISPER}/turbo-encoder.int8.onnx"),
                    674_716_297,
                    "b02dcdf54f348741e93fe732b67d933c8dcb6735655f710640143081db38878b",
                ),
                pinned(
                    "decoder.int8.onnx",
                    format!("{WHISPER}/turbo-decoder.int8.onnx"),
                    361_080_764,
                    "20accd02388482eb3a46bd615631adfdc85e1eb2c7db9ea3f02a40ffe6b81547",
                ),
                pinned(
                    "tokens.txt",
                    format!("{WHISPER}/turbo-tokens.txt"),
                    816_730,
                    "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126",
                ),
            ],
            credit: "OpenAI Whisper large-v3-turbo, MIT, quantized by sherpa-onnx".into(),
            licence_url: "https://github.com/openai/whisper/blob/main/LICENSE".into(),
            more_languages: None,
        },
        Model {
            id: "parakeet-tdt-110m-en".into(),
            name: "Parakeet English".into(),
            tag: Some("fast".into()),
            detail: "English only, small and quick".into(),
            engine: Engine::Transducer,
            files: vec![
                pinned(
                    "encoder.int8.onnx",
                    format!("{PARAKEET_ENGLISH}/encoder.int8.onnx"),
                    131_113_202,
                    "0f35509ddeb9b39002fb077d979a9fe74f06eb0bc4dd5c34f512f82e5111d657",
                ),
                pinned(
                    "decoder.int8.onnx",
                    format!("{PARAKEET_ENGLISH}/decoder.int8.onnx"),
                    3_955_863,
                    "f7c331c5504c2e593c76ed22b728e3f554af6c4a383dde862e719ced08b1da19",
                ),
                pinned(
                    "joiner.int8.onnx",
                    format!("{PARAKEET_ENGLISH}/joiner.int8.onnx"),
                    1_411_403,
                    "bf7dff69e9f2cdbe9943d70da358f38b361c115ba0105bae7e908e0d6ec782f6",
                ),
                pinned(
                    "tokens.txt",
                    format!("{PARAKEET_ENGLISH}/tokens.txt"),
                    9_953,
                    "450e56bd2f036fe5b6aa821865838cc5aa9d8b0106134ce9a9ba0664abe6cd10",
                ),
            ],
            credit: "NVIDIA Parakeet TDT 110M, CC BY 4.0, quantized by sherpa-onnx".into(),
            licence_url: CC_BY.into(),
            more_languages: Some("parakeet-tdt-0.6b-v3".into()),
        },
        Model {
            id: "moonshine-base-en".into(),
            name: "Moonshine".into(),
            tag: Some("English only".into()),
            detail: "English only".into(),
            engine: Engine::Moonshine,
            files: vec![
                pinned(
                    "preprocess.onnx",
                    format!("{MOONSHINE}/preprocess.onnx"),
                    14_077_290,
                    "ffa630d395c5ccf76f5d4954be5b882df76aaf6491519ec01fd82ea7a3819fb2",
                ),
                pinned(
                    "encode.int8.onnx",
                    format!("{MOONSHINE}/encode.int8.onnx"),
                    50_311_494,
                    "7e38770f776f2e5583a53b052936005df2ba5c833d7e09c2a5fd796b94bf73e2",
                ),
                pinned(
                    "uncached_decode.int8.onnx",
                    format!("{MOONSHINE}/uncached_decode.int8.onnx"),
                    122_120_451,
                    "c01f4b35093bcac20d352d23a75a539e772964579f9d024a90e5e6f09cae9987",
                ),
                pinned(
                    "cached_decode.int8.onnx",
                    format!("{MOONSHINE}/cached_decode.int8.onnx"),
                    99_983_837,
                    "2db74e51cedf64a8b1be3c8192e0bb5e4923af0e90bd9e87f8e8771873f8ea03",
                ),
                pinned(
                    "tokens.txt",
                    format!("{MOONSHINE}/tokens.txt"),
                    436_688,
                    "1165c2aeb9f72f457a83be2d459a09054f27490acd9b41bd43794dfd25e296ea",
                ),
            ],
            credit: "Useful Sensors Moonshine Base, MIT, quantized by sherpa-onnx".into(),
            licence_url: "https://github.com/usefulsensors/moonshine/blob/main/LICENSE".into(),
            more_languages: None,
        },
    ];
    if super::voice::BUILT {
        models.push(super::voice::offered());
    }
    models
}

/// One download under way.
struct Job {
    received: Arc<AtomicU64>,
    unpacking: Arc<std::sync::atomic::AtomicBool>,
    cancel: CancellationToken,
}

/// The desk's downloads: what is offered, what is under way, and why the
/// last one of each model failed.
pub struct Installs {
    root: PathBuf,
    catalogue: Vec<Model>,
    jobs: Mutex<HashMap<String, Job>>,
    failures: Mutex<HashMap<String, String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Where a build before 0.50 downloaded a model's archive, which a run it
/// did not finish may have left behind.
fn archive_path(root: &Path, id: &str) -> PathBuf {
    models_dir(root).join(format!("{id}.download"))
}
fn staging_path(root: &Path, id: &str) -> PathBuf {
    models_dir(root).join(format!("{id}.unpacking"))
}
fn removing_path(models: &Path, id: &str) -> PathBuf {
    models.join(format!("{id}.removing"))
}

impl Installs {
    /// Leftovers of a download a previous run did not finish are removed:
    /// nothing is resumed, because nothing unverified is kept.
    pub fn new(root: &Path, catalogue: Vec<Model>) -> Arc<Installs> {
        for model in &catalogue {
            let _ = std::fs::remove_file(archive_path(root, &model.id));
            let _ = std::fs::remove_dir_all(staging_path(root, &model.id));
            let _ = std::fs::remove_dir_all(removing_path(&models_dir(root), &model.id));
        }
        Arc::new(Installs {
            root: root.to_path_buf(),
            catalogue,
            jobs: Mutex::new(HashMap::new()),
            failures: Mutex::new(HashMap::new()),
        })
    }

    /// Every model offered, then any installed one the catalogue no longer lists.
    pub fn status(&self) -> Vec<SpeechModel> {
        let installed = installed(&self.root);
        let jobs = lock(&self.jobs);
        let failures = lock(&self.failures);
        let mut list: Vec<SpeechModel> = self
            .catalogue
            .iter()
            .map(|model| {
                let job = jobs.get(&model.id);
                let state = match job {
                    Some(job) if job.unpacking.load(Ordering::SeqCst) => {
                        SpeechModelState::Unpacking
                    }
                    Some(_) => SpeechModelState::Downloading,
                    None if installed.iter().any(|one| one.id == model.id) => {
                        SpeechModelState::Installed
                    }
                    None => SpeechModelState::Available,
                };
                SpeechModel {
                    id: model.id.clone(),
                    name: model.name.clone(),
                    tag: model.tag.clone(),
                    detail: model.detail.clone(),
                    download_bytes: model.bytes(),
                    disk_bytes: model.bytes(),
                    credit: model.credit.clone(),
                    licence_url: model.licence_url.clone(),
                    state,
                    received_bytes: job.map(|job| job.received.load(Ordering::SeqCst)),
                    error: failures.get(&model.id).cloned(),
                    more_languages: model.more_languages.clone(),
                }
            })
            .collect();
        for model in installed {
            if list.iter().all(|one| one.id != model.id) {
                list.push(SpeechModel {
                    id: model.id,
                    name: model.name,
                    tag: None,
                    detail: String::new(),
                    download_bytes: 0,
                    disk_bytes: 0,
                    credit: String::new(),
                    licence_url: String::new(),
                    state: SpeechModelState::Installed,
                    received_bytes: None,
                    error: None,
                    more_languages: None,
                });
            }
        }
        list
    }

    /// Starts downloading a model in the background; `status` follows it.
    pub fn install(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let model = self
            .catalogue
            .iter()
            .find(|model| model.id == id)
            .cloned()
            .ok_or_else(|| format!("Hotline offers no speech model called {id}."))?;
        if installed(&self.root).iter().any(|one| one.id == id) {
            return Ok(());
        }
        let job = Job {
            received: Arc::default(),
            unpacking: Arc::default(),
            cancel: CancellationToken::new(),
        };
        let (received, unpacking, cancel) = (
            job.received.clone(),
            job.unpacking.clone(),
            job.cancel.clone(),
        );
        {
            let mut jobs = lock(&self.jobs);
            if jobs.contains_key(id) {
                return Ok(());
            }
            jobs.insert(id.to_string(), job);
        }
        lock(&self.failures).remove(id);
        let this = self.clone();
        tokio::spawn(async move {
            let result = fetch(&this.root, &model, &received, &unpacking, &cancel).await;
            let _ = std::fs::remove_dir_all(staging_path(&this.root, &model.id));
            match result {
                Ok(()) => eprintln!("[voice] installed the speech model {}", model.id),
                Err(_) if cancel.is_cancelled() => {}
                Err(error) => {
                    eprintln!(
                        "[voice] the speech model {} did not install: {error}",
                        model.id
                    );
                    lock(&this.failures).insert(model.id.clone(), error);
                }
            }
            lock(&this.jobs).remove(&model.id);
        });
        Ok(())
    }

    /// Stops a download; what arrived is thrown away.
    pub fn cancel(&self, id: &str) {
        if let Some(job) = lock(&self.jobs).get(id) {
            job.cancel.cancel();
        }
    }

    /// Takes a model off the desk, or stops its download. Any call or
    /// dictation hearing with it finds it gone at its next utterance.
    pub fn remove(&self, id: &str) -> Result<(), String> {
        self.cancel(id);
        lock(&self.failures).remove(id);
        let Some(model) = installed(&self.root).into_iter().find(|one| one.id == id) else {
            return Ok(());
        };
        unload(&model.dir);
        remove_model(&model)
    }
}

fn remove_model(model: &Installed) -> Result<(), String> {
    // Renamed aside first, so a removal cut short never leaves a model that
    // looks whole but is missing a file.
    let aside = removing_path(model.dir.parent().unwrap_or(&model.dir), &model.id);
    std::fs::rename(&model.dir, &aside)
        .and_then(|()| std::fs::remove_dir_all(&aside))
        .map_err(|error| format!("The speech model could not be removed: {error}"))
}

/// Each of a model's files, downloaded straight into the staging directory
/// and refused unless it is the one pinned, then the model put in place.
async fn fetch(
    root: &Path,
    model: &Model,
    received: &AtomicU64,
    unpacking: &std::sync::atomic::AtomicBool,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("Hotline could not save the model: {error}");
    let staging = staging_path(root, &model.id);
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await.map_err(failed)?;
    let mut found = Vec::new();
    for file in &model.files {
        // A name is the desk's own, never a path: one of the engine's files.
        if !model.engine.files().contains(&file.name.as_str()) {
            return Err(mismatch());
        }
        download(
            &file.url,
            file.bytes,
            &file.sha256,
            &staging.join(&file.name),
            received,
            cancel,
        )
        .await?;
        found.push(FileHash {
            name: file.name.clone(),
            bytes: file.bytes,
            sha256: file.sha256.clone(),
        });
    }
    if found.len() != model.engine.files().len() {
        return Err(mismatch());
    }
    unpacking.store(true, Ordering::SeqCst);
    settle(root, model, &staging, found)
}

/// One file, hashed as it arrives and refused unless it is the one pinned:
/// `bytes` long with this `sha256`. What arrives counts towards `received`,
/// which a model of several files shares across them.
async fn download(
    url: &str,
    bytes: u64,
    sha256: &str,
    to: &Path,
    received: &AtomicU64,
    cancel: &CancellationToken,
) -> Result<(), String> {
    // Hugging Face answers with a redirect to its storage; a stalled body is
    // a failure, but a large one may take as long as the line needs.
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| "Hotline could not prepare the download.".to_string())?;
    let unreachable =
        |_| "The model could not be downloaded. Check the connection and try again.".to_string();
    let response = tokio::select! {
        () = cancel.cancelled() => return Err("Cancelled.".into()),
        response = client.get(url).send() => response.map_err(unreachable)?,
    };
    if !response.status().is_success() {
        return Err(format!(
            "The model's download answered HTTP {}. Try again later.",
            response.status().as_u16()
        ));
    }
    let mut file = tokio::fs::File::create(to)
        .await
        .map_err(|error| format!("Hotline could not save the model: {error}"))?;
    let mut hash = Sha256::new();
    let mut body = response.bytes_stream();
    let mut count: u64 = 0;
    loop {
        let chunk = tokio::select! {
            () = cancel.cancelled() => return Err("Cancelled.".into()),
            chunk = body.next() => chunk,
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(unreachable)?;
        count += chunk.len() as u64;
        if count > bytes {
            return Err(mismatch());
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("Hotline could not save the model: {error}"))?;
        received.fetch_add(chunk.len() as u64, Ordering::SeqCst);
    }
    file.sync_all()
        .await
        .map_err(|error| format!("Hotline could not save the model: {error}"))?;
    if count != bytes || hex::encode(hash.finalize()) != sha256 {
        return Err(mismatch());
    }
    Ok(())
}

fn mismatch() -> String {
    "The download was not the model Hotline expects, so it was thrown away.".into()
}

/// The staged model's manifest written, and the model renamed into place.
fn settle(root: &Path, model: &Model, staging: &Path, found: Vec<FileHash>) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("The model could not be unpacked: {error}");
    let manifest = Manifest {
        id: model.id.clone(),
        name: model.name.clone(),
        engine: model.engine,
        files: found,
    };
    std::fs::write(
        staging.join(MANIFEST),
        serde_json::to_vec_pretty(&manifest).expect("a manifest serializes"),
    )
    .map_err(failed)?;
    std::fs::rename(staging, models_dir(root).join(&model.id)).map_err(failed)
}

/// Copies `from` into `to` until it ends or the download is cancelled, and
/// answers how many bytes it was and their SHA-256.
pub(super) fn hash_copy(
    from: &mut impl Read,
    to: &mut impl Write,
    cancel: &CancellationToken,
) -> Result<(u64, String), String> {
    let mut buffer = vec![0; 1 << 20];
    let mut hash = Sha256::new();
    let mut bytes = 0;
    loop {
        if cancel.is_cancelled() {
            return Err("Cancelled.".into());
        }
        let read = from
            .read(&mut buffer)
            .map_err(|error| format!("The model could not be unpacked: {error}"))?;
        if read == 0 {
            return Ok((bytes, hex::encode(hash.finalize())));
        }
        hash.update(&buffer[..read]);
        bytes += read as u64;
        to.write_all(&buffer[..read])
            .map_err(|error| format!("The model could not be unpacked: {error}"))?;
    }
}
