//! Persist last template path under the app config directory (T1).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::dto::AppConfigDto;

const CONFIG_FILE: &str = "app-config.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct StoredConfig {
    template_path: Option<String>,
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("app config dir: {e}"))?;
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
    }
    Ok(dir.join(CONFIG_FILE))
}

fn load_stored(app: &AppHandle) -> Result<StoredConfig, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Ok(StoredConfig::default());
    }
    let bytes = fs::read(&path).map_err(|e| format!("read config: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse config: {e}"))
}

fn save_stored(app: &AppHandle, cfg: &StoredConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let bytes = serde_json::to_vec_pretty(cfg).map_err(|e| format!("serialize config: {e}"))?;
    fs::write(&path, bytes).map_err(|e| format!("write config: {e}"))
}

/// Load config; drop template path if the file no longer exists.
pub fn get_config(app: &AppHandle) -> Result<AppConfigDto, String> {
    let mut stored = load_stored(app)?;
    if let Some(ref p) = stored.template_path
        && !std::path::Path::new(p).is_file()
    {
        stored.template_path = None;
        // Best-effort cleanup of stale path.
        let _ = save_stored(app, &stored);
    }
    Ok(AppConfigDto {
        template_path: stored.template_path,
    })
}

pub fn set_template_path(app: &AppHandle, template_path: Option<String>) -> Result<(), String> {
    let mut stored = load_stored(app)?;
    stored.template_path = template_path.filter(|p| !p.trim().is_empty());
    save_stored(app, &stored)
}
