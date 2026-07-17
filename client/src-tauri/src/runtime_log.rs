use crate::error::{AppError, AppResult};
use chrono::Utc;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
#[cfg(unix)]
use std::{
    fs::Permissions,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub struct RuntimeLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl RuntimeLog {
    pub fn new(path: PathBuf) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() {
            restrict_permissions(&path)?;
        }
        Ok(Self {
            path,
            lock: Mutex::new(()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn record(
        &self,
        level: &str,
        event: &str,
        message: impl Into<String>,
        data: Value,
    ) -> AppResult<()> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| AppError::Runtime("运行日志锁已损坏".into()))?;
        self.rotate_if_needed()?;
        let entry = json!({
            "timestamp": Utc::now().to_rfc3339(),
            "level": level,
            "event": event,
            "message": message.into(),
            "data": data,
        });
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&self.path)?;
        restrict_permissions(&self.path)?;
        serde_json::to_writer(&mut file, &entry)?;
        file.write_all(b"\n")?;
        file.flush()?;
        Ok(())
    }

    fn rotate_if_needed(&self) -> AppResult<()> {
        if fs::metadata(&self.path)
            .map(|metadata| metadata.len() >= MAX_LOG_BYTES)
            .unwrap_or(false)
        {
            let previous = self.path.with_file_name("runtime.previous.jsonl");
            if previous.exists() {
                fs::remove_file(&previous)?;
            }
            fs::rename(&self.path, previous)?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> std::io::Result<()> {
    fs::set_permissions(path, Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_one_json_object_per_line() {
        let directory = std::env::temp_dir().join(format!(
            "lumadrobe-runtime-log-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let path = directory.join("runtime.jsonl");
        let log = RuntimeLog::new(path.clone()).expect("create log");
        log.record("info", "test", "hello", json!({ "targetCount": 1 }))
            .expect("write log");

        let contents = fs::read_to_string(&path).expect("read log");
        let entry: Value = serde_json::from_str(contents.trim()).expect("parse log line");
        assert_eq!(entry["event"], "test");
        assert_eq!(entry["data"]["targetCount"], 1);

        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).expect("read metadata").permissions().mode() & 0o777,
            0o600
        );

        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
