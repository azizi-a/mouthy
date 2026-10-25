//! Settings → Models: download, cancel and delete the offline voice's files.
//!
//! Downloads run as tasks on the async runtime. Each reports
//! `model-download-progress` while it runs, and `model-files-changed` is sent
//! whenever a file starts, stops or goes and after every preload of the local
//! voice, so the page re-reads the list.

use super::models_dir;
use crate::settings::{self, ModelVariant};
use crate::tts::catalog::{self, CatalogFile, ModelFileKind, VoiceLanguage};
use crate::tts::download;
use crate::tts::local::{self, ModelFiles};
use crate::tts::voice::VoiceSpec;
use futures_util::future::{AbortHandle, AbortRegistration, Abortable};
use log::{info, warn};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

/// The downloads in flight and the last failure of each file. Locked briefly
/// by commands and download tasks, never across an await.
#[derive(Default)]
pub struct ModelDownloads(Mutex<DownloadTable>);

#[derive(Default)]
struct DownloadTable {
    running: HashMap<String, Running>,
    failed: HashMap<String, String>,
}

/// A file's entry stays until its task ends, so a file has one task at most.
struct Running {
    abort: AbortHandle,
    downloaded: u64,
}

impl ModelDownloads {
    fn lock(&self) -> MutexGuard<'_, DownloadTable> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Serialize, Type)]
pub struct ModelFileList {
    /// The folder the files live in.
    pub dir: String,
    pub files: Vec<ModelFile>,
    /// The selected model, if its file is not there.
    pub missing_model: Option<ModelVariant>,
    /// The selected voices the local backend finds no file for, or the whole
    /// setting when it is not a voice or blend.
    pub missing_voices: Vec<String>,
    /// The selected voice, or each voice of the selected blend.
    pub voice_parts: Vec<VoicePart>,
    /// Why the local voice last failed to load, if it did.
    pub load_error: Option<String>,
}

#[derive(Serialize, Type)]
pub struct VoicePart {
    pub id: String,
    /// Its share of the blend, from 0 to 1.
    pub share: f32,
}

#[derive(Serialize, Type)]
pub struct ModelFile {
    /// A model variant (`int8`) or a voice (`bm_lewis`).
    pub id: String,
    pub kind: ModelFileKind,
    pub size: u64,
    pub installed: bool,
    /// Bytes downloaded so far: a running download's, or a stopped one's that
    /// the next download resumes from.
    pub partial: u64,
    pub downloading: bool,
    /// Why the last download failed, until the next one starts.
    pub error: Option<String>,
    /// A voice's language; `None` for a model.
    pub language: Option<VoiceLanguage>,
}

#[derive(Clone, Serialize)]
struct DownloadProgress<'a> {
    id: &'a str,
    downloaded: u64,
    total: u64,
}

#[tauri::command]
#[specta::specta]
pub async fn list_model_files(app: AppHandle) -> Result<ModelFileList, String> {
    let models_dir = models_dir(&app)?;
    let (running, failed): (HashMap<String, u64>, HashMap<String, String>) = {
        let downloads = app.state::<ModelDownloads>();
        let table = downloads.lock();
        let running = table
            .running
            .iter()
            .map(|(id, r)| (id.clone(), r.downloaded))
            .collect();
        (running, table.failed.clone())
    };
    let files = catalog::all(&models_dir)
        .into_iter()
        .map(|file| ModelFile {
            installed: file.path.is_file(),
            partial: running
                .get(&file.id)
                .copied()
                .unwrap_or_else(|| partial_len(&file.path)),
            downloading: running.contains_key(&file.id),
            error: failed.get(&file.id).cloned(),
            size: file.size,
            kind: file.kind,
            language: file.language,
            id: file.id,
        })
        .collect();
    let settings = settings::get_settings(&app);
    let selected = ModelFiles::new(&models_dir, settings.tts_model_variant, &settings.tts_voice);
    let voice_parts = VoiceSpec::parse(&settings.tts_voice)
        .map(|spec| {
            spec.shares()
                .map(|(id, share)| VoicePart {
                    id: id.to_string(),
                    share,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ModelFileList {
        dir: local::kokoro_dir(&models_dir)
            .to_string_lossy()
            .into_owned(),
        files,
        missing_model: selected
            .model_missing()
            .then_some(settings.tts_model_variant),
        missing_voices: selected.missing_voices(),
        voice_parts,
        load_error: local::preload_error(),
    })
}

/// Starts downloading a file, or resumes its partial download. Returns at
/// once; a file already downloading is left alone.
#[tauri::command]
#[specta::specta]
pub async fn download_model_file(app: AppHandle, id: String) -> Result<(), String> {
    let file = find(&app, &id)?;
    let client = download::client()?;
    let (abort, registration) = AbortHandle::new_pair();
    {
        let downloads = app.state::<ModelDownloads>();
        let mut table = downloads.lock();
        if table.running.contains_key(&id) {
            return Ok(());
        }
        table.failed.remove(&id);
        let downloaded = partial_len(&file.path);
        table.running.insert(id, Running { abort, downloaded });
    }
    files_changed(&app);
    tauri::async_runtime::spawn(run_download(app, file, client, registration));
    Ok(())
}

/// Stops a download, keeping what arrived for the next one to resume.
#[tauri::command]
#[specta::specta]
pub async fn cancel_model_download(app: AppHandle, id: String) -> Result<(), String> {
    if let Some(running) = app.state::<ModelDownloads>().lock().running.get(&id) {
        running.abort.abort();
    }
    Ok(())
}

/// Deletes a file and any partial download of it. A running speech session
/// keeps its loaded copy; the model is unloaded once its files are gone.
#[tauri::command]
#[specta::specta]
pub async fn delete_model_file(app: AppHandle, id: String) -> Result<(), String> {
    let file = find(&app, &id)?;
    {
        let downloads = app.state::<ModelDownloads>();
        // Held while deleting, so a download of this file cannot start meanwhile.
        let mut table = downloads.lock();
        if table.running.contains_key(&id) {
            return Err(format!("{id} is downloading; cancel it first"));
        }
        download::remove_if_present(&file.path)?;
        download::remove_if_present(&download::part_path(&file.path))?;
        table.failed.remove(&id);
    }
    info!("Deleted {}", file.path.display());
    files_changed(&app);
    crate::actions::preload_local_voice(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn open_models_dir(app: AppHandle) -> Result<(), String> {
    let dir = local::kokoro_dir(&models_dir(&app)?);
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<String>)
        .map_err(|e| format!("Failed to open the models folder: {e}"))
}

fn find(app: &AppHandle, id: &str) -> Result<CatalogFile, String> {
    catalog::find(&models_dir(app)?, id).ok_or_else(|| format!("Unknown model file '{id}'"))
}

fn partial_len(dest: &Path) -> u64 {
    fs::metadata(download::part_path(dest)).map_or(0, |m| m.len())
}

pub(crate) fn files_changed(app: &AppHandle) {
    if let Err(e) = app.emit("model-files-changed", ()) {
        warn!("Failed to report a model file change: {e}");
    }
}

async fn run_download(
    app: AppHandle,
    file: CatalogFile,
    client: reqwest::Client,
    registration: AbortRegistration,
) {
    let mut finish = Finish {
        app: app.clone(),
        id: file.id.clone(),
        error: Some("The download stopped unexpectedly".into()),
    };
    let progress = |downloaded: u64| {
        if let Some(running) = app
            .state::<ModelDownloads>()
            .lock()
            .running
            .get_mut(&file.id)
        {
            running.downloaded = downloaded;
        }
        let event = DownloadProgress {
            id: &file.id,
            downloaded,
            total: file.size,
        };
        if let Err(e) = app.emit("model-download-progress", event) {
            warn!("Failed to report download progress: {e}");
        }
    };
    info!("Downloading {} from {}", file.id, file.url);
    let started = Instant::now();
    let fetch = download::download(
        &client,
        &file.url,
        &file.path,
        file.size,
        file.sha256,
        &progress,
    );
    match Abortable::new(fetch, registration).await {
        Ok(Ok(())) => {
            info!(
                "Downloaded and verified {} in {:.1} s",
                file.path.display(),
                started.elapsed().as_secs_f32()
            );
            finish.error = None;
            crate::actions::preload_local_voice(&app);
        }
        Ok(Err(e)) => {
            warn!("Download of {} failed: {e}", file.id);
            finish.error = Some(e);
        }
        Err(_) => {
            info!("Download of {} cancelled", file.id);
            finish.error = None;
        }
    }
}

/// Clears a download's entry however its task ends, a panic included, so its
/// row never stays on "downloading".
struct Finish {
    app: AppHandle,
    id: String,
    error: Option<String>,
}

impl Drop for Finish {
    fn drop(&mut self) {
        {
            let downloads = self.app.state::<ModelDownloads>();
            let mut table = downloads.lock();
            table.running.remove(&self.id);
            if let Some(error) = self.error.take() {
                table.failed.insert(self.id.clone(), error);
            }
        }
        files_changed(&self.app);
    }
}
