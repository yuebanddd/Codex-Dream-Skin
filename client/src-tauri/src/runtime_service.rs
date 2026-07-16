use crate::atomic_file::replace_file;
use crate::cdp;
use crate::codex_process::CodexInstall;
use crate::error::{AppError, AppResult};
use crate::installed_store::InstalledStore;
use crate::models::{InstalledSkin, RuntimeStatus};
use crate::renderer_payload::build_payload;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{watch, Mutex};
use tokio::time::{interval, sleep, Instant};

const RUNTIME_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeRecord {
    schema_version: u32,
    source_id: String,
    skin_id: String,
    version: String,
    port: u16,
    browser_id: String,
    platform: String,
    executable: String,
    codex_identity: String,
}

struct RuntimeInner {
    status: RuntimeStatus,
    record: Option<RuntimeRecord>,
    install: Option<CodexInstall>,
    child: Option<Child>,
    cancel: Option<watch::Sender<bool>>,
    generation: u64,
}

pub struct RuntimeManager {
    http: Client,
    path: PathBuf,
    inner: Mutex<RuntimeInner>,
}

impl RuntimeManager {
    pub fn new(path: PathBuf) -> AppResult<Arc<Self>> {
        let record = if path.exists() {
            let parsed: RuntimeRecord = serde_json::from_slice(&std::fs::read(&path)?)?;
            if parsed.schema_version != RUNTIME_SCHEMA {
                return Err(AppError::Runtime("本地皮肤引擎状态版本不受支持".into()));
            }
            Some(parsed)
        } else {
            None
        };
        let status = record
            .as_ref()
            .map(|record| status_for_record("checking", record, "正在验证上次的皮肤会话"))
            .unwrap_or_else(|| RuntimeStatus::stopped("皮肤引擎已就绪"));
        Ok(Arc::new(Self {
            http: cdp::build_client()?,
            path,
            inner: Mutex::new(RuntimeInner {
                status,
                record,
                install: None,
                child: None,
                cancel: None,
                generation: 0,
            }),
        }))
    }

    pub async fn status(&self) -> RuntimeStatus {
        self.inner.lock().await.status.clone()
    }

    pub async fn is_active_theme(&self, source_id: &str, skin_id: &str) -> bool {
        let inner = self.inner.lock().await;
        inner
            .record
            .as_ref()
            .is_some_and(|record| record.source_id == source_id && record.skin_id == skin_id)
            && matches!(
                inner.status.phase.as_str(),
                "running" | "checking" | "error"
            )
    }

    pub async fn recover(self: &Arc<Self>, installed_store: Arc<Mutex<InstalledStore>>) {
        let record = {
            let inner = self.inner.lock().await;
            inner.record.clone()
        };
        let Some(record) = record else {
            return;
        };
        let result = self.recover_inner(&record, installed_store).await;
        if let Err(error) = result {
            let mut inner = self.inner.lock().await;
            if inner
                .record
                .as_ref()
                .is_some_and(|current| current.browser_id == record.browser_id)
            {
                inner.status =
                    status_for_record("error", &record, format!("上次的皮肤会话需要处理：{error}"));
            }
        }
    }

    async fn recover_inner(
        self: &Arc<Self>,
        record: &RuntimeRecord,
        installed_store: Arc<Mutex<InstalledStore>>,
    ) -> AppResult<()> {
        let install = CodexInstall::discover()?;
        validate_saved_install(&install, record)?;
        verify_endpoint(&self.http, &install, record).await?;
        let installed = installed_store
            .lock()
            .await
            .get(&record.source_id, &record.skin_id)
            .filter(|skin| skin.version == record.version)
            .ok_or_else(|| AppError::Runtime("上次应用的主题版本已不在本地主题库".into()))?;
        let payload = build_payload(&installed)?;
        cdp::apply_to_verified_targets(&self.http, record.port, &record.browser_id, &payload)
            .await?;
        self.start_watcher(record.clone(), install, None, payload)
            .await?;
        Ok(())
    }

    pub async fn apply(self: &Arc<Self>, installed: InstalledSkin) -> AppResult<RuntimeStatus> {
        let payload = build_payload(&installed)?;
        {
            let mut inner = self.inner.lock().await;
            inner.status = RuntimeStatus {
                phase: "starting".into(),
                active_source_id: Some(installed.source_id.clone()),
                active_skin_id: Some(installed.skin_id.clone()),
                active_version: Some(installed.version.clone()),
                port: inner.record.as_ref().map(|record| record.port),
                message: "正在验证 Codex 并启动回环 CDP".into(),
            };
        }

        let (previous_record, previous_install, mut previous_child) = {
            let mut inner = self.inner.lock().await;
            if let Some(cancel) = inner.cancel.take() {
                let _ = cancel.send(true);
            }
            (
                inner.record.clone(),
                inner.install.clone(),
                inner.child.take(),
            )
        };

        let mut reuse = None;
        if let (Some(record), Some(install)) = (&previous_record, &previous_install) {
            if validate_saved_install(install, record).is_ok()
                && verify_endpoint(&self.http, install, record).await.is_ok()
            {
                reuse = Some((record.clone(), install.clone()));
            }
        }

        let (record, install, child) = if let Some((active, install)) = reuse {
            cdp::apply_to_verified_targets(&self.http, active.port, &active.browser_id, &payload)
                .await?;
            (
                RuntimeRecord {
                    source_id: installed.source_id.clone(),
                    skin_id: installed.skin_id.clone(),
                    version: installed.version.clone(),
                    ..active
                },
                install,
                previous_child,
            )
        } else {
            if let Some(install) = &previous_install {
                if previous_child.is_some() {
                    install.stop(previous_child.as_mut())?;
                }
            }
            let install = CodexInstall::discover()?;
            if install.is_running()? {
                return self
                    .fail_start(
                        &installed,
                        "Codex 正在运行但没有可验证的 LumaDrobe CDP 会话；请完全退出 Codex 后重试",
                    )
                    .await;
            }
            let port = select_available_port(install.preferred_port())?;
            let mut child = install.launch_with_cdp(port)?;
            let ready = wait_until_ready(&self.http, &install, port).await;
            let (browser_id, _) = match ready {
                Ok(ready) => ready,
                Err(error) => {
                    let _ = install.stop(Some(&mut child));
                    let _ = install.launch_normally();
                    return self.fail_start(&installed, error.to_string()).await;
                }
            };
            let record = RuntimeRecord {
                schema_version: RUNTIME_SCHEMA,
                source_id: installed.source_id.clone(),
                skin_id: installed.skin_id.clone(),
                version: installed.version.clone(),
                port,
                browser_id,
                platform: install.platform.clone(),
                executable: install.executable.to_string_lossy().into_owned(),
                codex_identity: install.identity.clone(),
            };
            if let Err(error) = cdp::apply_to_verified_targets(
                &self.http,
                record.port,
                &record.browser_id,
                &payload,
            )
            .await
            {
                let _ = install.stop(Some(&mut child));
                let _ = install.launch_normally();
                return self.fail_start(&installed, error.to_string()).await;
            }
            (record, install, Some(child))
        };

        self.persist(Some(&record))?;
        self.start_watcher(record, install, child, payload).await?;
        Ok(self.status().await)
    }

    pub async fn restore(self: &Arc<Self>) -> AppResult<RuntimeStatus> {
        let (record, install, mut child) = {
            let mut inner = self.inner.lock().await;
            if let Some(cancel) = inner.cancel.take() {
                let _ = cancel.send(true);
            }
            inner.status.phase = "stopping".into();
            inner.status.message = "正在移除注入并关闭 CDP 会话".into();
            (
                inner.record.clone(),
                inner.install.clone(),
                inner.child.take(),
            )
        };
        let Some(record) = record else {
            let status = RuntimeStatus::stopped("当前没有运行中的皮肤会话");
            self.inner.lock().await.status = status.clone();
            return Ok(status);
        };

        let install = match install {
            Some(install) => install,
            None => CodexInstall::discover()?,
        };
        let verified = validate_saved_install(&install, &record).is_ok()
            && verify_endpoint(&self.http, &install, &record).await.is_ok();
        if !verified {
            self.persist(None)?;
            let status = RuntimeStatus::stopped(
                "记录中的 CDP 会话已经不存在；未触碰任何未验证进程，状态已清理",
            );
            let mut inner = self.inner.lock().await;
            inner.record = None;
            inner.install = None;
            inner.status = status.clone();
            return Ok(status);
        }

        let _ =
            cdp::remove_from_verified_targets(&self.http, record.port, &record.browser_id).await;
        if let Err(error) = install.stop(child.as_mut()) {
            let mut inner = self.inner.lock().await;
            inner.record = Some(record.clone());
            inner.install = Some(install);
            inner.child = child;
            inner.status = status_for_record("error", &record, error.to_string());
            return Err(error);
        }
        install.launch_normally()?;
        self.persist(None)?;
        let status = RuntimeStatus::stopped("已恢复 Codex 原生外观并重新启动");
        let mut inner = self.inner.lock().await;
        inner.record = None;
        inner.install = None;
        inner.child = None;
        inner.status = status.clone();
        Ok(status)
    }

    async fn start_watcher(
        self: &Arc<Self>,
        record: RuntimeRecord,
        install: CodexInstall,
        child: Option<Child>,
        payload: String,
    ) -> AppResult<()> {
        let (cancel, mut cancelled) = watch::channel(false);
        let generation = {
            let mut inner = self.inner.lock().await;
            inner.generation += 1;
            let generation = inner.generation;
            inner.status = status_for_record(
                "running",
                &record,
                format!(
                    "皮肤引擎运行中 · {} · 端口 {}",
                    install.platform, record.port
                ),
            );
            inner.record = Some(record.clone());
            inner.install = Some(install.clone());
            inner.child = child;
            inner.cancel = Some(cancel);
            generation
        };
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            let mut ticker = interval(Duration::from_secs(2));
            let mut failures = 0_u8;
            let mut ticks = 0_u8;
            loop {
                tokio::select! {
                    changed = cancelled.changed() => {
                        if changed.is_err() || *cancelled.borrow() { break; }
                    }
                    _ = ticker.tick() => {
                        ticks = ticks.wrapping_add(1);
                        let owner_ok = ticks % 5 != 0 || install.verify_listener_owner(record.port).unwrap_or(false);
                        let applied = owner_ok && cdp::apply_to_verified_targets(
                            &manager.http,
                            record.port,
                            &record.browser_id,
                            &payload,
                        ).await.is_ok();
                        if applied {
                            failures = 0;
                        } else {
                            failures = failures.saturating_add(1);
                        }
                        if failures >= 3 {
                            manager.mark_watcher_error(generation, &record).await;
                            break;
                        }
                    }
                }
            }
        });
        Ok(())
    }

    async fn mark_watcher_error(&self, generation: u64, record: &RuntimeRecord) {
        let mut inner = self.inner.lock().await;
        if inner.generation == generation {
            inner.cancel = None;
            inner.status = status_for_record(
                "error",
                record,
                "CDP 会话连续失联；为防止端口被其他进程复用，已停止重注入",
            );
        }
    }

    async fn fail_start<T>(
        &self,
        installed: &InstalledSkin,
        message: impl Into<String>,
    ) -> AppResult<T> {
        let message = message.into();
        let _ = self.persist(None);
        let mut inner = self.inner.lock().await;
        inner.record = None;
        inner.install = None;
        inner.child = None;
        inner.status = RuntimeStatus {
            phase: "error".into(),
            active_source_id: Some(installed.source_id.clone()),
            active_skin_id: Some(installed.skin_id.clone()),
            active_version: Some(installed.version.clone()),
            port: None,
            message: message.clone(),
        };
        Err(AppError::Runtime(message))
    }

    fn persist(&self, record: Option<&RuntimeRecord>) -> AppResult<()> {
        if let Some(record) = record {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let temporary = self.path.with_extension("json.tmp");
            std::fs::write(&temporary, serde_json::to_vec_pretty(record)?)?;
            replace_file(&temporary, &self.path)?;
        } else if self.path.exists() {
            std::fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

fn status_for_record(
    phase: &str,
    record: &RuntimeRecord,
    message: impl Into<String>,
) -> RuntimeStatus {
    RuntimeStatus {
        phase: phase.into(),
        active_source_id: Some(record.source_id.clone()),
        active_skin_id: Some(record.skin_id.clone()),
        active_version: Some(record.version.clone()),
        port: Some(record.port),
        message: message.into(),
    }
}

fn validate_saved_install(install: &CodexInstall, record: &RuntimeRecord) -> AppResult<()> {
    if install.platform != record.platform
        || install.identity != record.codex_identity
        || !install.same_executable(&record.executable)
    {
        return Err(AppError::Runtime(
            "已保存的 Codex 应用身份与当前官方安装不一致".into(),
        ));
    }
    Ok(())
}

async fn verify_endpoint(
    http: &Client,
    install: &CodexInstall,
    record: &RuntimeRecord,
) -> AppResult<()> {
    if !install.verify_listener_owner(record.port)? {
        return Err(AppError::Runtime("CDP 端口不再属于已验证的 Codex".into()));
    }
    let identity = cdp::browser_identity(http, record.port).await?;
    if identity.id != record.browser_id {
        return Err(AppError::Runtime("CDP 浏览器身份已经改变".into()));
    }
    Ok(())
}

fn select_available_port(preferred: u16) -> AppResult<u16> {
    let upper = preferred.saturating_add(100);
    for port in preferred..=upper {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err(AppError::Runtime(format!(
        "端口 {preferred}–{upper} 均不可用"
    )))
}

async fn wait_until_ready(
    http: &Client,
    install: &CodexInstall,
    port: u16,
) -> AppResult<(String, usize)> {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut last_error = "Codex 尚未开放 CDP".to_string();
    while Instant::now() < deadline {
        match cdp::browser_identity(http, port).await {
            Ok(identity) => {
                if !install.verify_listener_owner(port)? {
                    return Err(AppError::Runtime(
                        "CDP 端口监听者不是已验证的官方 Codex".into(),
                    ));
                }
                match cdp::verified_targets(http, port, &identity.id).await {
                    Ok(targets) if !targets.is_empty() => return Ok((identity.id, targets.len())),
                    Ok(_) => last_error = "CDP 尚未出现 app:// 页面".into(),
                    Err(error) => last_error = error.to_string(),
                }
            }
            Err(error) => last_error = error.to_string(),
        }
        sleep(Duration::from_millis(350)).await;
    }
    Err(AppError::Runtime(format!(
        "等待 Codex CDP 超时：{last_error}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_status_keeps_the_active_theme_identity() {
        let record = RuntimeRecord {
            schema_version: 1,
            source_id: "source".into(),
            skin_id: "night".into(),
            version: "1.2.3".into(),
            port: 9341,
            browser_id: "browser".into(),
            platform: "macos".into(),
            executable: "/Applications/ChatGPT.app/test".into(),
            codex_identity: "identity".into(),
        };
        let status = status_for_record("running", &record, "ok");
        assert_eq!(status.active_source_id.as_deref(), Some("source"));
        assert_eq!(status.active_skin_id.as_deref(), Some("night"));
        assert_eq!(status.port, Some(9341));
    }

    #[test]
    fn saved_identity_must_match_exactly() {
        let install = CodexInstall {
            platform: "macos".into(),
            executable: PathBuf::from("/Applications/ChatGPT.app/test"),
            bundle_path: None,
            version: "1".into(),
            identity: "official".into(),
        };
        let mut record = RuntimeRecord {
            schema_version: 1,
            source_id: "source".into(),
            skin_id: "night".into(),
            version: "1".into(),
            port: 9341,
            browser_id: "browser".into(),
            platform: "macos".into(),
            executable: "/Applications/ChatGPT.app/test".into(),
            codex_identity: "official".into(),
        };
        assert!(validate_saved_install(&install, &record).is_ok());
        record.codex_identity = "other".into();
        assert!(validate_saved_install(&install, &record).is_err());
    }
}
