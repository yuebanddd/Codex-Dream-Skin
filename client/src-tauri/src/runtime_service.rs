use crate::atomic_file::replace_file;
use crate::cdp;
use crate::codex_process::CodexInstall;
use crate::error::{AppError, AppResult};
use crate::installed_store::InstalledStore;
use crate::models::{InstalledSkin, RuntimeDiagnostics, RuntimeStatus};
use crate::renderer_payload::build_payload;
use crate::runtime_log::RuntimeLog;
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Arc;
use std::time::Duration;
use tauri::async_runtime::JoinHandle;
use tokio::sync::{watch, Mutex};
use tokio::time::{interval, sleep, Instant};

const RUNTIME_SCHEMA: u32 = 1;
const READINESS_SNAPSHOT_LEAD: Duration = Duration::from_secs(35);

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
    #[serde(default)]
    paused: bool,
}

struct RuntimeInner {
    status: RuntimeStatus,
    record: Option<RuntimeRecord>,
    install: Option<CodexInstall>,
    child: Option<Child>,
    cancel: Option<watch::Sender<bool>>,
    watcher: Option<JoinHandle<()>>,
    payload: Option<String>,
    generation: u64,
}

pub struct RuntimeManager {
    http: Client,
    path: PathBuf,
    log: RuntimeLog,
    inner: Mutex<RuntimeInner>,
}

impl RuntimeManager {
    pub fn new(path: PathBuf) -> AppResult<Arc<Self>> {
        let data_directory = path
            .parent()
            .ok_or_else(|| AppError::Runtime("无法定位 LumaDrobe 数据目录".into()))?;
        let log = RuntimeLog::new(data_directory.join("logs").join("runtime.jsonl"))?;
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
        let saved_session = record.is_some();
        let manager = Arc::new(Self {
            http: cdp::build_client()?,
            path,
            log,
            inner: Mutex::new(RuntimeInner {
                status,
                record,
                install: None,
                child: None,
                cancel: None,
                watcher: None,
                payload: None,
                generation: 0,
            }),
        });
        manager.record_log(
            "info",
            "runtime_started",
            "LumaDrobe runtime initialized",
            json!({
                "clientVersion": env!("CARGO_PKG_VERSION"),
                "buildCommit": option_env!("LUMADROBE_BUILD_SHA").unwrap_or("development"),
                "platform": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "savedSession": saved_session,
            }),
        );
        Ok(manager)
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
                "running" | "paused" | "pausing" | "checking" | "error"
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
            self.record_log(
                "error",
                "recovery_failed",
                error.to_string(),
                record_log_data(&record),
            );
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
        if pending_launch(record) {
            return Err(AppError::Runtime(
                "上次的 Codex CDP 启动未完成；请先恢复原生后重试".into(),
            ));
        }
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
        if record.paused {
            cdp::remove_from_verified_targets(&self.http, record.port, &record.browser_id).await?;
            let mut inner = self.inner.lock().await;
            inner.record = Some(record.clone());
            inner.install = Some(install);
            inner.payload = Some(payload);
            inner.status = status_for_record(
                "paused",
                record,
                "皮肤已暂停 · Codex 与回环 CDP 会话保持运行",
            );
            return Ok(());
        }
        if let Err(error) =
            cdp::apply_to_verified_targets(&self.http, record.port, &record.browser_id, &payload)
                .await
        {
            self.spawn_cdp_failure_snapshot(
                "cdp_recovery_probe_failed",
                error.to_string(),
                record.port,
                Some(record.browser_id.clone()),
            );
            return Err(error);
        }
        self.start_watcher(record.clone(), install, None, payload)
            .await;
        Ok(())
    }

    pub async fn apply(self: &Arc<Self>, installed: InstalledSkin) -> AppResult<RuntimeStatus> {
        self.record_log(
            "info",
            "theme_apply_requested",
            "Theme apply requested",
            json!({
                "sourceId": installed.source_id,
                "skinId": installed.skin_id,
                "version": installed.version,
            }),
        );
        let payload = match build_payload(&installed) {
            Ok(payload) => payload,
            Err(error) => {
                self.record_log(
                    "error",
                    "payload_build_failed",
                    error.to_string(),
                    json!({
                        "sourceId": installed.source_id,
                        "skinId": installed.skin_id,
                        "version": installed.version,
                    }),
                );
                return Err(error);
            }
        };
        self.record_log(
            "info",
            "payload_ready",
            "Renderer payload validated",
            json!({ "payloadBytes": payload.len() }),
        );
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

        self.stop_watcher().await;
        let (previous_record, previous_install, mut previous_child) = {
            let mut inner = self.inner.lock().await;
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
        if reuse.is_none() {
            if let Some(record) = &previous_record {
                let fresh_install = match CodexInstall::discover() {
                    Ok(install) => install,
                    Err(error) => {
                        return self
                            .fail_apply_attempt(
                                &installed,
                                previous_record.clone(),
                                previous_install.clone(),
                                previous_child,
                                error.to_string(),
                            )
                            .await;
                    }
                };
                if validate_saved_install(&fresh_install, record).is_ok()
                    && verify_endpoint(&self.http, &fresh_install, record)
                        .await
                        .is_ok()
                {
                    reuse = Some((record.clone(), fresh_install));
                }
            }
        }

        let (record, install, child) = if let Some((active, install)) = reuse {
            let applied_targets = match cdp::apply_to_verified_targets(
                &self.http,
                active.port,
                &active.browser_id,
                &payload,
            )
            .await
            {
                Ok(count) => count,
                Err(error) => {
                    let failure = error.to_string();
                    self.spawn_cdp_failure_snapshot(
                        "cdp_apply_probe_failed",
                        failure.clone(),
                        active.port,
                        Some(active.browser_id.clone()),
                    );
                    return self
                        .fail_apply_attempt(
                            &installed,
                            Some(active),
                            Some(install),
                            previous_child,
                            failure,
                        )
                        .await;
                }
            };
            self.record_log(
                "info",
                "theme_injected",
                "Theme injected into existing Codex session",
                json!({
                    "port": active.port,
                    "browserId": active.browser_id,
                    "verifiedTargets": applied_targets,
                }),
            );
            (
                RuntimeRecord {
                    source_id: installed.source_id.clone(),
                    skin_id: installed.skin_id.clone(),
                    version: installed.version.clone(),
                    paused: false,
                    ..active
                },
                install,
                previous_child,
            )
        } else {
            if previous_child.is_some() {
                if let Some(install) = previous_install.clone() {
                    if let Err(error) = install.stop(previous_child.as_mut()) {
                        return self
                            .fail_apply_attempt(
                                &installed,
                                previous_record.clone(),
                                Some(install),
                                previous_child,
                                error.to_string(),
                            )
                            .await;
                    }
                }
            }
            if let Some(record) = previous_record.clone() {
                match CodexInstall::saved_executable_is_running(
                    &record.platform,
                    &record.executable,
                ) {
                    Ok(true) => {
                        return self
                            .fail_apply_attempt(
                                &installed,
                                Some(record),
                                previous_install.clone(),
                                previous_child,
                                "已保存路径对应的 Codex 仍在运行，但原 CDP 会话无法验证；为避免启动第二个会话，状态已保留，请先恢复或完全退出旧进程",
                            )
                            .await;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        return self
                            .fail_apply_attempt(
                                &installed,
                                Some(record),
                                previous_install.clone(),
                                previous_child,
                                format!("无法确认旧 Codex 进程是否仍在运行：{error}"),
                            )
                            .await;
                    }
                }
            }
            let install = match CodexInstall::discover() {
                Ok(install) => install,
                Err(error) => {
                    return self
                        .fail_apply_attempt(
                            &installed,
                            previous_record.clone(),
                            previous_install.clone(),
                            previous_child,
                            error.to_string(),
                        )
                        .await;
                }
            };
            let is_running = match install.is_running() {
                Ok(is_running) => is_running,
                Err(error) => {
                    return self
                        .fail_apply_attempt(
                            &installed,
                            previous_record.clone(),
                            Some(install),
                            previous_child,
                            error.to_string(),
                        )
                        .await;
                }
            };
            if is_running {
                if previous_record.is_some() {
                    return self
                        .fail_apply_attempt(
                            &installed,
                            previous_record.clone(),
                            Some(install),
                            previous_child,
                            "Codex 正在运行，但现有 CDP 会话暂时无法重新验证；会话记录已保留，请重试或恢复原生",
                        )
                        .await;
                }
                return self
                    .fail_start(
                        &installed,
                        "Codex 正在运行但没有可验证的 LumaDrobe CDP 会话；请完全退出 Codex 后重试",
                    )
                    .await;
            }
            let port = match select_available_port(install.preferred_port()) {
                Ok(port) => port,
                Err(error) => {
                    return self
                        .fail_apply_attempt(
                            &installed,
                            previous_record.clone(),
                            Some(install),
                            previous_child,
                            error.to_string(),
                        )
                        .await;
                }
            };
            self.record_log(
                "info",
                "codex_launch_requested",
                "Launching verified Codex installation with loopback CDP",
                json!({
                    "platform": install.platform,
                    "codexIdentity": install.identity,
                    "executable": install.executable,
                    "port": port,
                }),
            );
            let child = match install.launch_with_cdp(port) {
                Ok(child) => child,
                Err(error) => {
                    return self
                        .fail_apply_attempt(
                            &installed,
                            previous_record.clone(),
                            Some(install),
                            previous_child,
                            error.to_string(),
                        )
                        .await;
                }
            };
            let mut record = RuntimeRecord {
                schema_version: RUNTIME_SCHEMA,
                source_id: installed.source_id.clone(),
                skin_id: installed.skin_id.clone(),
                version: installed.version.clone(),
                port,
                browser_id: String::new(),
                platform: install.platform.clone(),
                executable: install.executable.to_string_lossy().into_owned(),
                codex_identity: install.identity.clone(),
                paused: false,
            };
            let ready = wait_until_ready_and_apply(self, &install, port, &payload).await;
            let (browser_id, applied_targets) = match ready {
                Ok(ready) => ready,
                Err(error) => {
                    return self
                        .abort_failed_launch(&installed, record, install, child, error.to_string())
                        .await;
                }
            };
            self.record_log(
                "info",
                "theme_injected",
                "Theme injected into newly launched Codex session",
                json!({
                    "port": port,
                    "browserId": browser_id,
                    "verifiedTargets": applied_targets,
                }),
            );
            record.browser_id = browser_id;
            (record, install, Some(child))
        };

        if let Err(error) = self.persist(Some(&record)) {
            return self
                .fail_apply_attempt(
                    &installed,
                    Some(record),
                    Some(install),
                    child,
                    format!("主题已经注入，但运行状态无法保存：{error}"),
                )
                .await;
        }
        self.start_watcher(record, install, child, payload).await;
        Ok(self.status().await)
    }

    pub async fn pause(self: &Arc<Self>) -> AppResult<RuntimeStatus> {
        let (record, install, payload) = {
            let inner = self.inner.lock().await;
            (
                inner.record.clone(),
                inner.install.clone(),
                inner.payload.clone(),
            )
        };
        let Some(record) = record else {
            return Err(AppError::Runtime("当前没有可暂停的皮肤会话".into()));
        };
        if record.paused {
            return Ok(self.status().await);
        }
        if pending_launch(&record) {
            return Err(AppError::Runtime(
                "Codex CDP 启动尚未完成，请先恢复原生".into(),
            ));
        }
        let payload = payload.ok_or_else(|| AppError::Runtime("活动主题载荷不可用".into()))?;
        let install = match install {
            Some(install) => install,
            None => CodexInstall::discover()?,
        };
        validate_saved_install(&install, &record)?;
        verify_endpoint(&self.http, &install, &record).await?;
        {
            let mut inner = self.inner.lock().await;
            inner.status = status_for_record("pausing", &record, "正在停止重注入并移除主题");
        }
        self.stop_watcher().await;
        let child = self.inner.lock().await.child.take();
        let mut paused_record = record.clone();
        paused_record.paused = true;
        if let Err(error) = self.persist(Some(&paused_record)) {
            self.start_watcher(record, install, child, payload).await;
            return Err(AppError::Runtime(format!("暂停状态无法保存：{error}")));
        }
        if let Err(remove_error) = cdp::remove_from_verified_targets(
            &self.http,
            paused_record.port,
            &paused_record.browser_id,
        )
        .await
        {
            if let Err(rollback_error) = self.persist(Some(&record)) {
                let message =
                    format!("主题移除失败：{remove_error}；活动状态也无法回滚：{rollback_error}");
                let mut inner = self.inner.lock().await;
                inner.record = Some(paused_record.clone());
                inner.install = Some(install);
                inner.child = child;
                inner.payload = Some(payload);
                inner.status = status_for_record("error", &paused_record, &message);
                return Err(AppError::Runtime(message));
            }
            self.start_watcher(record, install, child, payload).await;
            return Err(AppError::Runtime(format!("无法暂停皮肤：{remove_error}")));
        }
        let status = status_for_record(
            "paused",
            &paused_record,
            "皮肤已暂停 · Codex 与回环 CDP 会话保持运行",
        );
        let mut inner = self.inner.lock().await;
        inner.record = Some(paused_record);
        inner.install = Some(install);
        inner.child = child;
        inner.payload = Some(payload);
        inner.status = status.clone();
        Ok(status)
    }

    pub async fn resume(self: &Arc<Self>) -> AppResult<RuntimeStatus> {
        let (record, install, payload) = {
            let inner = self.inner.lock().await;
            (
                inner.record.clone(),
                inner.install.clone(),
                inner.payload.clone(),
            )
        };
        let Some(record) = record else {
            return Err(AppError::Runtime("当前没有可恢复的暂停会话".into()));
        };
        if !record.paused {
            return Ok(self.status().await);
        }
        let payload = payload.ok_or_else(|| AppError::Runtime("暂停主题载荷不可用".into()))?;
        let install = match install {
            Some(install) => install,
            None => CodexInstall::discover()?,
        };
        validate_saved_install(&install, &record)?;
        verify_endpoint(&self.http, &install, &record).await?;
        if let Err(error) =
            cdp::apply_to_verified_targets(&self.http, record.port, &record.browser_id, &payload)
                .await
        {
            let failure = error.to_string();
            self.spawn_cdp_failure_snapshot(
                "cdp_resume_probe_failed",
                failure.clone(),
                record.port,
                Some(record.browser_id.clone()),
            );
            let message = format!("恢复主题失败：{failure}");
            self.inner.lock().await.status = status_for_record("paused", &record, &message);
            return Err(AppError::Runtime(message));
        }
        let mut active_record = record.clone();
        active_record.paused = false;
        if let Err(error) = self.persist(Some(&active_record)) {
            let rollback =
                cdp::remove_from_verified_targets(&self.http, record.port, &record.browser_id)
                    .await;
            let (phase, message) = match rollback {
                Ok(_) => (
                    "paused",
                    format!("恢复状态无法保存，已回到暂停状态：{error}"),
                ),
                Err(rollback_error) => (
                    "error",
                    format!(
                        "主题已注入但恢复状态无法保存：{error}；回滚移除也失败：{rollback_error}"
                    ),
                ),
            };
            self.inner.lock().await.status = status_for_record(phase, &record, &message);
            return Err(AppError::Runtime(message));
        }
        let child = self.inner.lock().await.child.take();
        self.start_watcher(active_record, install, child, payload)
            .await;
        Ok(self.status().await)
    }

    pub async fn diagnostics(&self) -> RuntimeDiagnostics {
        let (runtime, record, managed_install) = {
            let inner = self.inner.lock().await;
            (
                inner.status.clone(),
                inner.record.clone(),
                inner.install.clone(),
            )
        };
        let mut notes = Vec::new();
        let install = match managed_install {
            Some(install) => Some(install),
            None => match CodexInstall::discover() {
                Ok(install) => Some(install),
                Err(error) => {
                    notes.push(error.to_string());
                    None
                }
            },
        };
        let codex_running = install
            .as_ref()
            .and_then(|install| match install.is_running() {
                Ok(running) => Some(running),
                Err(error) => {
                    notes.push(format!("Codex 进程检查失败：{error}"));
                    None
                }
            });
        let mut listener_verified = None;
        let mut endpoint_verified = None;
        let mut verified_targets = None;
        if let (Some(record), Some(install)) = (&record, &install) {
            if pending_launch(record) {
                notes.push("保存的 CDP 启动尚未完成".into());
            } else if validate_saved_install(install, record).is_err() {
                listener_verified = Some(false);
                endpoint_verified = Some(false);
                notes.push("保存的会话与当前官方 Codex 安装不一致".into());
            } else {
                let owner_ok = install.verify_listener_owner(record.port).unwrap_or(false);
                listener_verified = Some(owner_ok);
                let endpoint_ok =
                    owner_ok && verify_endpoint(&self.http, install, record).await.is_ok();
                endpoint_verified = Some(endpoint_ok);
                if endpoint_ok {
                    match cdp::count_codex_targets(&self.http, record.port, &record.browser_id)
                        .await
                    {
                        Ok(count) => verified_targets = Some(count),
                        Err(error) => notes.push(format!("渲染页检查失败：{error}")),
                    }
                }
            }
        }
        RuntimeDiagnostics {
            generated_at: Utc::now().to_rfc3339(),
            log_path: self.log.path().to_string_lossy().into_owned(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            build_commit: option_env!("LUMADROBE_BUILD_SHA")
                .unwrap_or("development")
                .into(),
            platform: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            runtime,
            saved_session: record.is_some(),
            paused: record.as_ref().is_some_and(|record| record.paused),
            codex_found: install.is_some(),
            codex_version: install.as_ref().map(|install| install.version.clone()),
            codex_identity: install.as_ref().map(|install| install.identity.clone()),
            executable: install
                .as_ref()
                .map(|install| install.executable.to_string_lossy().into_owned()),
            codex_running,
            listener_verified,
            endpoint_verified,
            verified_targets,
            notes,
        }
    }

    pub async fn export_diagnostics(&self) -> AppResult<String> {
        let diagnostics = self.diagnostics().await;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| AppError::Runtime("无法定位 LumaDrobe 数据目录".into()))?;
        let directory = parent.join("diagnostics");
        std::fs::create_dir_all(&directory)?;
        let filename = format!(
            "lumadrobe-diagnostics-{}.json",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        );
        let path = directory.join(filename);
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&diagnostics)?)?;
        replace_file(&temporary, &path)?;
        Ok(path.to_string_lossy().into_owned())
    }

    pub async fn restore(self: &Arc<Self>) -> AppResult<RuntimeStatus> {
        self.stop_watcher().await;
        let (record, install, mut child) = {
            let mut inner = self.inner.lock().await;
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
            None => match CodexInstall::discover() {
                Ok(install) => install,
                Err(error) => {
                    return self
                        .fail_recoverable_session(record, None, child, error.to_string())
                        .await;
                }
            },
        };
        let install_matches_saved = validate_saved_install(&install, &record).is_ok();
        let verified = install_matches_saved
            && if pending_launch(&record) {
                install.is_running().unwrap_or(false)
            } else {
                verify_endpoint(&self.http, &install, &record).await.is_ok()
            };
        if !verified {
            let saved_process_running =
                CodexInstall::saved_executable_is_running(&record.platform, &record.executable);
            if saved_process_running.unwrap_or(true) {
                return self
                    .fail_recoverable_session(
                        record,
                        install_matches_saved.then_some(install),
                        child,
                        "已保存的 Codex 会话暂时无法验证，但旧进程仍可能运行；状态已保留，请完全退出该进程后重试恢复",
                    )
                    .await;
            }
            if let Err(error) = self.persist(None) {
                return self
                    .fail_recoverable_session(
                        record,
                        Some(install),
                        child,
                        format!("会话已失效，但运行状态无法清理：{error}"),
                    )
                    .await;
            }
            let status = RuntimeStatus::stopped(
                "记录中的 CDP 会话已经不存在；未触碰任何未验证进程，状态已清理",
            );
            let mut inner = self.inner.lock().await;
            inner.record = None;
            inner.install = None;
            inner.child = None;
            inner.payload = None;
            inner.status = status.clone();
            return Ok(status);
        }

        if !pending_launch(&record) {
            let _ = cdp::remove_from_verified_targets(&self.http, record.port, &record.browser_id)
                .await;
        }
        if let Err(error) = install.stop(child.as_mut()) {
            let mut inner = self.inner.lock().await;
            inner.record = Some(record.clone());
            inner.install = Some(install);
            inner.child = child;
            inner.status = status_for_record("error", &record, error.to_string());
            return Err(error);
        }
        if let Err(error) = install.launch_normally() {
            return self
                .fail_recoverable_session(record, Some(install), child, error.to_string())
                .await;
        }
        if let Err(error) = self.persist(None) {
            return self
                .fail_recoverable_session(
                    record,
                    Some(install),
                    None,
                    format!("Codex 已恢复原生启动，但运行状态无法清理：{error}"),
                )
                .await;
        }
        let status = RuntimeStatus::stopped("已恢复 Codex 原生外观并重新启动");
        let mut inner = self.inner.lock().await;
        inner.record = None;
        inner.install = None;
        inner.child = None;
        inner.payload = None;
        inner.status = status.clone();
        Ok(status)
    }

    async fn start_watcher(
        self: &Arc<Self>,
        record: RuntimeRecord,
        install: CodexInstall,
        child: Option<Child>,
        payload: String,
    ) {
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
            inner.payload = Some(payload.clone());
            generation
        };
        let manager = Arc::clone(self);
        let theme_key = format!("{}:{}@{}", record.source_id, record.skin_id, record.version);
        self.record_log(
            "info",
            "watcher_started",
            "Renderer reinjection watcher started",
            json!({
                "port": record.port,
                "browserId": record.browser_id,
                "themeKey": theme_key,
            }),
        );
        let watcher = tauri::async_runtime::spawn(async move {
            let mut ticker = interval(Duration::from_secs(2));
            let mut failures = 0_u8;
            loop {
                tokio::select! {
                    changed = cancelled.changed() => {
                        if changed.is_err() || *cancelled.borrow() { break; }
                    }
                    _ = ticker.tick() => {
                        let result = match install.verify_listener_owner(record.port) {
                            Ok(true) => cdp::ensure_theme_on_verified_targets(
                                &manager.http,
                                record.port,
                                &record.browser_id,
                                &theme_key,
                                &payload,
                            ).await.map(|_| ()),
                            Ok(false) => Err(AppError::Runtime(
                                "CDP 监听进程不再属于已验证的 Codex".into(),
                            )),
                            Err(error) => Err(error),
                        };
                        if result.is_ok() {
                            if failures > 0 {
                                manager.record_log(
                                    "info",
                                    "watcher_recovered",
                                    "Renderer watcher recovered after a transient failure",
                                    json!({ "consecutiveFailures": failures }),
                                );
                            }
                            failures = 0;
                        } else {
                            failures = failures.saturating_add(1);
                            let error = result.expect_err("failed watcher check must contain error");
                            if failures == 1 {
                                manager.record_log(
                                    "warn",
                                    "watcher_check_failed",
                                    error.to_string(),
                                    json!({
                                        "port": record.port,
                                        "browserId": record.browser_id,
                                        "consecutiveFailures": failures,
                                    }),
                                );
                            }
                            if failures >= 3 {
                                let failure = error.to_string();
                                manager
                                    .mark_watcher_error(generation, &record, &failure)
                                    .await;
                                manager
                                    .spawn_cdp_failure_snapshot(
                                        "cdp_watcher_probe_failed",
                                        failure,
                                        record.port,
                                        Some(record.browser_id.clone()),
                                    );
                                break;
                            }
                        }
                    }
                }
            }
        });
        let mut inner = self.inner.lock().await;
        if inner.generation == generation {
            inner.watcher = Some(watcher);
        } else {
            watcher.abort();
        }
    }

    async fn stop_watcher(&self) {
        let (cancel, watcher) = {
            let mut inner = self.inner.lock().await;
            inner.generation += 1;
            (inner.cancel.take(), inner.watcher.take())
        };
        if let Some(cancel) = cancel {
            let _ = cancel.send(true);
        }
        if let Some(watcher) = watcher {
            let _ = watcher.await;
        }
    }

    async fn mark_watcher_error(&self, generation: u64, record: &RuntimeRecord, failure: &str) {
        self.record_log("error", "watcher_stopped", failure, record_log_data(record));
        let mut inner = self.inner.lock().await;
        if inner.generation == generation {
            inner.cancel = None;
            inner.watcher = None;
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
        self.record_log(
            "error",
            "theme_apply_failed",
            message.clone(),
            json!({
                "sourceId": installed.source_id,
                "skinId": installed.skin_id,
                "version": installed.version,
            }),
        );
        let _ = self.persist(None);
        let mut inner = self.inner.lock().await;
        inner.record = None;
        inner.install = None;
        inner.child = None;
        inner.payload = None;
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

    async fn abort_failed_launch<T>(
        &self,
        installed: &InstalledSkin,
        record: RuntimeRecord,
        install: CodexInstall,
        mut child: Child,
        failure: impl Into<String>,
    ) -> AppResult<T> {
        let failure = failure.into();
        if let Err(stop_error) = install.stop(Some(&mut child)) {
            let persist_error = self.persist(Some(&record)).err();
            let message = match persist_error {
                Some(persist_error) => format!(
                    "{failure}；停止 CDP Codex 也失败：{stop_error}；恢复记录无法保存：{persist_error}"
                ),
                None => format!(
                    "{failure}；停止 CDP Codex 也失败：{stop_error}；未完成会话已保留，请使用恢复原生重试"
                ),
            };
            return self
                .fail_recoverable_session(record, Some(install), Some(child), message)
                .await;
        }
        if let Err(relaunch_error) = install.launch_normally() {
            return self
                .fail_start(
                    installed,
                    format!("{failure}；CDP Codex 已停止，但恢复普通启动失败：{relaunch_error}"),
                )
                .await;
        }
        self.fail_start(installed, failure).await
    }

    async fn fail_apply_attempt<T>(
        &self,
        installed: &InstalledSkin,
        record: Option<RuntimeRecord>,
        install: Option<CodexInstall>,
        child: Option<Child>,
        message: impl Into<String>,
    ) -> AppResult<T> {
        let message = message.into();
        let Some(record) = record else {
            return self.fail_start(installed, message).await;
        };
        self.fail_recoverable_session(record, install, child, message)
            .await
    }

    async fn fail_recoverable_session<T>(
        &self,
        record: RuntimeRecord,
        install: Option<CodexInstall>,
        child: Option<Child>,
        message: impl Into<String>,
    ) -> AppResult<T> {
        let message = message.into();
        self.record_log(
            "error",
            "runtime_session_failed",
            message.clone(),
            record_log_data(&record),
        );
        let mut inner = self.inner.lock().await;
        inner.record = Some(record.clone());
        inner.install = install;
        inner.child = child;
        inner.status = status_for_record("error", &record, &message);
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

    fn record_log(&self, level: &str, event: &str, message: impl Into<String>, data: Value) {
        let _ = self.log.record(level, event, message, data);
    }

    fn spawn_cdp_failure_snapshot(
        self: &Arc<Self>,
        event: &'static str,
        message: String,
        port: u16,
        browser_id: Option<String>,
    ) {
        let manager = Arc::clone(self);
        tauri::async_runtime::spawn(async move {
            manager
                .record_cdp_failure_snapshot(event, &message, port, browser_id.as_deref())
                .await;
        });
    }

    async fn record_cdp_failure_snapshot(
        &self,
        event: &str,
        message: &str,
        port: u16,
        browser_id: Option<&str>,
    ) {
        let snapshot = cdp::diagnostic_snapshot(&self.http, port, browser_id).await;
        self.record_log(
            "error",
            event,
            message,
            json!({
                "port": port,
                "browserId": browser_id,
                "snapshot": snapshot,
            }),
        );
    }
}

fn record_log_data(record: &RuntimeRecord) -> Value {
    json!({
        "sourceId": record.source_id,
        "skinId": record.skin_id,
        "version": record.version,
        "port": record.port,
        "browserId": record.browser_id,
        "platform": record.platform,
        "codexIdentity": record.codex_identity,
        "executable": record.executable,
        "paused": record.paused,
    })
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

fn pending_launch(record: &RuntimeRecord) -> bool {
    record.browser_id.is_empty()
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

async fn wait_until_ready_and_apply(
    manager: &Arc<RuntimeManager>,
    install: &CodexInstall,
    port: u16,
    payload: &str,
) -> AppResult<(String, usize)> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut last_error = "Codex 尚未开放 CDP".to_string();
    let mut last_browser_id = None;
    let mut pending_snapshot: Option<JoinHandle<Value>> = None;
    while Instant::now() < deadline {
        match cdp::browser_identity(&manager.http, port).await {
            Ok(identity) => {
                last_browser_id = Some(identity.id.clone());
                if !install.verify_listener_owner(port)? {
                    return Err(AppError::Runtime(
                        "CDP 端口监听者不是已验证的官方 Codex".into(),
                    ));
                }
                match cdp::apply_to_verified_targets(
                    &manager.http,
                    port,
                    &identity.id,
                    payload,
                )
                .await
                {
                    Ok(count) if count > 0 => {
                        if let Some(snapshot) = pending_snapshot.take() {
                            snapshot.abort();
                        }
                        return Ok((identity.id, count));
                    }
                    Ok(_) => last_error = "Codex 渲染页尚未完成 DOM 初始化".into(),
                    Err(error) => last_error = error.to_string(),
                }
            }
            Err(error) => last_error = error.to_string(),
        }
        if pending_snapshot.is_none()
            && last_browser_id.is_some()
            && deadline.saturating_duration_since(Instant::now()) <= READINESS_SNAPSHOT_LEAD
        {
            // Capture while the launched process is still alive, then consume the result off-path.
            let http = manager.http.clone();
            let expected_browser_id = last_browser_id.clone();
            pending_snapshot = Some(tauri::async_runtime::spawn(async move {
                cdp::diagnostic_snapshot(&http, port, expected_browser_id.as_deref()).await
            }));
        }
        sleep(Duration::from_millis(350)).await;
    }
    let manager = Arc::clone(manager);
    let diagnostic_browser_id = last_browser_id.clone();
    let diagnostic_error = last_error.clone();
    tauri::async_runtime::spawn(async move {
        let snapshot = match pending_snapshot {
            Some(snapshot) => snapshot.await.unwrap_or_else(|error| {
                json!({
                    "port": port,
                    "stage": "snapshotTask",
                    "error": error.to_string(),
                })
            }),
            None => {
                cdp::diagnostic_snapshot(
                    &manager.http,
                    port,
                    diagnostic_browser_id.as_deref(),
                )
                .await
            }
        };
        manager.record_log(
            "error",
            "cdp_readiness_timeout",
            format!(
                "Codex renderer did not become injectable before the deadline: {diagnostic_error}"
            ),
            json!({
                "port": port,
                "browserId": diagnostic_browser_id,
                "snapshot": snapshot,
            }),
        );
    });
    Err(AppError::Runtime(format!(
        "等待 Codex CDP 与主题注入超时：{last_error}"
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
            paused: false,
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
            paused: false,
        };
        assert!(validate_saved_install(&install, &record).is_ok());
        record.codex_identity = "other".into();
        assert!(validate_saved_install(&install, &record).is_err());
    }

    #[test]
    fn empty_browser_identity_marks_an_unfinished_launch() {
        let record = RuntimeRecord {
            schema_version: 1,
            source_id: "source".into(),
            skin_id: "night".into(),
            version: "1".into(),
            port: 9341,
            browser_id: String::new(),
            platform: "macos".into(),
            executable: "/Applications/Codex.app/test".into(),
            codex_identity: "official".into(),
            paused: false,
        };
        assert!(pending_launch(&record));
    }

    #[test]
    fn legacy_runtime_record_defaults_to_active() {
        let record: RuntimeRecord = serde_json::from_value(serde_json::json!({
            "schemaVersion": 1,
            "sourceId": "source",
            "skinId": "night",
            "version": "1",
            "port": 9341,
            "browserId": "browser",
            "platform": "macos",
            "executable": "/Applications/Codex.app/test",
            "codexIdentity": "official"
        }))
        .unwrap();
        assert!(!record.paused);
    }

    #[tokio::test]
    async fn transient_failure_keeps_a_saved_session_recoverable() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("lumadrobe-runtime-{}-{unique}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("runtime.json");
        let record = RuntimeRecord {
            schema_version: 1,
            source_id: "source".into(),
            skin_id: "night".into(),
            version: "1".into(),
            port: 9341,
            browser_id: "browser".into(),
            platform: "macos".into(),
            executable: "/Applications/ChatGPT.app/test".into(),
            codex_identity: "official".into(),
            paused: false,
        };
        std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
        let manager = RuntimeManager::new(path.clone()).unwrap();
        let install = CodexInstall {
            platform: "macos".into(),
            executable: PathBuf::from(&record.executable),
            bundle_path: None,
            version: "1".into(),
            identity: "official".into(),
        };

        let result: AppResult<()> = manager
            .fail_recoverable_session(record.clone(), Some(install), None, "transient")
            .await;

        assert!(result.is_err());
        assert!(path.exists());
        let inner = manager.inner.lock().await;
        assert_eq!(inner.record.as_ref().unwrap().browser_id, record.browser_id);
        assert_eq!(inner.status.phase, "error");
        drop(inner);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
