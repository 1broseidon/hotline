//! Downloading the desk's speech models, which the owner asks for one at a
//! time: nothing is installed until they do.
//!
//! A model that hears is one archive from sherpa-onnx's releases, pinned here
//! by its size and SHA-256. The archive is hashed as it arrives and refused
//! whole if it does not match, before any of it is unpacked; from a verified
//! archive only the model's own files are taken, by name, so nothing in it
//! chooses a path. The desk's voice is several files from Hugging Face, each
//! pinned the same way and refused the same way ([`Source::Files`]). Either
//! way the model appears only when its directory is complete, by renaming it
//! into place, so a crash or a cancel leaves no half model to load.

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
    pub source: Source,
    /// Everything downloaded: the archive, or the files together.
    pub download_bytes: u64,
    pub disk_bytes: u64,
    pub credit: String,
    pub licence_url: String,
    /// A model offered here that hears more languages, by id, for Settings
    /// to suggest to someone whose window is not in English while this is
    /// the only one installed.
    pub more_languages: Option<String>,
}

/// Where a model's files come from.
#[derive(Clone, Debug)]
pub enum Source {
    /// One archive, whose files are taken out by name.
    Archive {
        url: String,
        /// What the archive's file names carry before the desk's own names:
        /// Whisper's are `turbo-encoder.int8.onnx` and so on.
        prefix: String,
        /// Lowercase hex of the archive's SHA-256.
        sha256: String,
    },
    /// Files downloaded one by one, each kept under its own name.
    Files(Vec<Pinned>),
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

const RELEASES: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models";
const CC_BY: &str = "https://creativecommons.org/licenses/by/4.0/";

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
            source: Source::Archive {
                url: format!("{RELEASES}/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2"),
                prefix: String::new(),
                sha256: "5793d0fd397c5778d2cf2126994d58e9d56b1be7c04d13c7a15bb1b4eafb16bf".into(),
            },
            download_bytes: 487_170_055,
            disk_bytes: 670_478_772,
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
            source: Source::Archive {
                url: format!("{RELEASES}/sherpa-onnx-whisper-turbo.tar.bz2"),
                prefix: "turbo-".into(),
                sha256: "b11acbbcd660b44a8e0df33724feb5aaa709cf65668f2823d59f656312544f22".into(),
            },
            download_bytes: 563_790_207,
            disk_bytes: 1_036_613_791,
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
            source: Source::Archive {
                url: format!(
                    "{RELEASES}/sherpa-onnx-nemo-parakeet_tdt_transducer_110m-en-36000-int8.tar.bz2"
                ),
                prefix: String::new(),
                sha256: "f628312e9fdf8686374cb01a69425c41732529d540860311f16f37cbc32cfe9b".into(),
            },
            download_bytes: 108_035_095,
            disk_bytes: 136_490_421,
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
            source: Source::Archive {
                url: format!("{RELEASES}/sherpa-onnx-moonshine-base-en-int8.tar.bz2"),
                prefix: String::new(),
                sha256: "21870cecaa2e44e4e2bf63e02d1072bed183ccd10284871353bd9d24dad14e5e".into(),
            },
            download_bytes: 250_807_309,
            disk_bytes: 286_929_760,
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

/// What a download is called while it is not yet a model.
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
                    download_bytes: model.download_bytes,
                    disk_bytes: model.disk_bytes,
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
            let _ = std::fs::remove_file(archive_path(&this.root, &model.id));
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

/// Downloads, verifies and unpacks one model into place.
async fn fetch(
    root: &Path,
    model: &Model,
    received: &AtomicU64,
    unpacking: &std::sync::atomic::AtomicBool,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let dir = models_dir(root);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| format!("Hotline could not make a place for the model: {error}"))?;
    let (url, prefix, sha256) = match &model.source {
        Source::Archive {
            url,
            prefix,
            sha256,
        } => (url, prefix.clone(), sha256),
        Source::Files(files) => {
            return fetch_files(root, model, files, received, unpacking, cancel).await;
        }
    };
    let archive = archive_path(root, &model.id);
    download(
        url,
        model.download_bytes,
        sha256,
        &archive,
        received,
        cancel,
    )
    .await?;
    unpacking.store(true, Ordering::SeqCst);
    let (root, model, cancel) = (root.to_path_buf(), model.clone(), cancel.clone());
    tokio::task::spawn_blocking(move || unpack(&root, &model, &prefix, &archive, &cancel))
        .await
        .map_err(|_| "Unpacking the model stopped.".to_string())?
}

/// Each of a model's files, downloaded straight into the staging directory
/// and refused unless it is the one pinned, then the model put in place.
async fn fetch_files(
    root: &Path,
    model: &Model,
    files: &[Pinned],
    received: &AtomicU64,
    unpacking: &std::sync::atomic::AtomicBool,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("Hotline could not save the model: {error}");
    let staging = staging_path(root, &model.id);
    let _ = tokio::fs::remove_dir_all(&staging).await;
    tokio::fs::create_dir_all(&staging).await.map_err(failed)?;
    let mut found = Vec::new();
    for file in files {
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
    // GitHub answers with a redirect to its storage; a stalled body is a
    // failure, but a large one may take as long as the line needs.
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

/// The model's files out of a verified archive, into a directory that is
/// renamed into place once it is whole. Each name in the archive is
/// `prefix` and then one of the engine's files.
fn unpack(
    root: &Path,
    model: &Model,
    prefix: &str,
    archive: &Path,
    cancel: &CancellationToken,
) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("The model could not be unpacked: {error}");
    let staging = staging_path(root, &model.id);
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(failed)?;
    let file = std::fs::File::open(archive).map_err(failed)?;
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(std::io::BufReader::new(file)));
    let files = model.engine.files();
    let mut found: Vec<FileHash> = Vec::new();
    for entry in tar.entries().map_err(failed)? {
        let mut entry = entry.map_err(failed)?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(failed)?.into_owned();
        let Some(name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(prefix))
            .and_then(|name| files.iter().find(|file| **file == name))
        else {
            continue;
        };
        // An archive that names a file twice keeps the first.
        if found.iter().any(|file| file.name == *name) {
            continue;
        }
        let mut out = std::fs::File::create(staging.join(name)).map_err(failed)?;
        let (bytes, sha256) = hash_copy(&mut entry, &mut out, cancel)?;
        out.sync_all().map_err(failed)?;
        found.push(FileHash {
            name: name.to_string(),
            bytes,
            sha256,
        });
    }
    if found.len() != files.len() {
        return Err("The download did not hold the model's files.".into());
    }
    settle(root, model, &staging, found)
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
