use crate::cdp_transfer::ThemeTransferPlan;
use crate::error::{AppError, AppResult};
use crate::renderer_payload::RendererPayload;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::{sleep, timeout, timeout_at, Instant};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const RENDERER_PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const EVALUATE_TIMEOUT: Duration = Duration::from_secs(30);
const TRANSFER_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const TRANSFER_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const INSTALL_START_TIMEOUT: Duration = Duration::from_secs(5);
const INSTALL_POLL_TIMEOUT: Duration = Duration::from_secs(3);
const INSTALL_DEADLINE: Duration = Duration::from_secs(25);
const INSTALL_POLL_INTERVAL: Duration = Duration::from_millis(125);
const TARGET_STABILITY_DELAY: Duration = Duration::from_millis(300);
const DIAGNOSTIC_PROBE_TIMEOUT: Duration = Duration::from_secs(12);
const DIAGNOSTIC_TARGET_LIMIT: usize = 16;
const DIAGNOSTIC_PROBE_LIMIT: usize = 4;

const PROBE_EXPRESSION: &str = r#"(() => {
  const clip = (value, limit = 80) => typeof value === 'string' ? value.slice(0, limit) : null;
  const describe = (node) => ({
    tag: node.tagName?.toLowerCase() ?? null,
    id: clip(node.id),
    role: clip(node.getAttribute?.('role')),
    testId: clip(node.getAttribute?.('data-testid')),
    classes: Array.from(node.classList ?? []).slice(0, 8).map((value) => clip(value, 64)),
  });
  const markers = {
    shell: Boolean(document.querySelector('main.main-surface')),
    sidebar: Boolean(document.querySelector('aside.app-shell-left-panel')),
    composer: Boolean(document.querySelector('.composer-surface-chrome')),
    main: Boolean(document.querySelector('[role="main"]')),
  };
  const security = {
    appProtocol: location.protocol === 'app:',
    documentReady: Boolean(document.documentElement && document.head && document.body) &&
      (document.readyState === 'interactive' || document.readyState === 'complete'),
  };
  const landmarks = Array.from(document.querySelectorAll(
    'main,aside,nav,header,[role="main"],[role="navigation"],[data-testid]'
  )).slice(0, 24).map(describe);
  const elementCount = document.getElementsByTagName('*').length;
  const viewportWidth = Math.max(0, Math.round(window.innerWidth || 0));
  const viewportHeight = Math.max(0, Math.round(window.innerHeight || 0));
  const surfaceReady = markers.shell || markers.sidebar || markers.composer || markers.main ||
    Boolean(document.querySelector('main'));
  const visibilityState = document.visibilityState || 'unknown';
  const presentable = security.documentReady && document.readyState === 'complete' &&
    visibilityState === 'visible' && viewportWidth >= 320 && viewportHeight >= 240 &&
    surfaceReady && elementCount >= 16;
  return {
    // Rust has already verified the signed Codex process, listener owner, browser
    // identity, app:// target URL and loopback WebSocket path. Keep only the
    // renderer-local, navigation-safe checks here; private CSS class names are
    // diagnostic hints and must not become a compatibility gate.
    codex: security.appProtocol && security.documentReady,
    security,
    document: {
      protocol: location.protocol,
      pathname: clip(location.pathname, 160),
      readyState: document.readyState,
      hasDocumentElement: Boolean(document.documentElement),
      hasHead: Boolean(document.head),
      hasBody: Boolean(document.body),
      elementCount,
      navigationEpoch: Number.isFinite(performance.timeOrigin) ? Math.round(performance.timeOrigin) : null,
      root: document.documentElement ? describe(document.documentElement) : null,
      body: document.body ? describe(document.body) : null,
    },
    presentation: {
      presentable,
      visibilityState,
      hasFocus: document.hasFocus(),
      viewportWidth,
      viewportHeight,
      surfaceReady,
    },
    markers,
    landmarks,
  };
})()"#;

pub const REMOVE_EXPRESSION: &str = r#"(() => {
  const state = window.__LUMADROBE_RUNTIME__;
  if (state?.cleanup) return state.cleanup();
  document.documentElement?.classList.remove('lumadrobe-theme', 'codex-dream-skin');
  for (const property of ['--lumadrobe-art', '--dream-art', '--dream-skin-art']) {
    document.documentElement?.style.removeProperty(property);
  }
  document.documentElement?.removeAttribute('data-dream-shell');
  document.querySelectorAll('.lumadrobe-surface, .dream-home, .dream-skin-home, .dream-home-shell, .dream-skin-home-shell')
    .forEach((node) => node.classList.remove('lumadrobe-surface', 'dream-home', 'dream-skin-home', 'dream-home-shell', 'dream-skin-home-shell'));
  document.getElementById('lumadrobe-theme-style')?.remove();
  delete window.__LUMADROBE_RUNTIME__;
  return true;
})()"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CdpTarget {
    pub id: String,
    #[serde(rename = "type")]
    pub target_type: String,
    pub url: String,
    pub web_socket_debugger_url: String,
}

#[derive(Debug, Clone)]
pub struct BrowserIdentity {
    pub id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionResponse {
    web_socket_debugger_url: String,
}

type PageSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

const RENDERER_SESSION_DOMAINS: [&str; 2] = ["Runtime.enable", "Page.enable"];

struct CdpSession {
    endpoint: String,
    stream: PageSocket,
    next_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CdpTransport {
    DirectPage,
    BrowserSession,
}

impl CdpTransport {
    fn as_str(self) -> &'static str {
        match self {
            Self::DirectPage => "directPage",
            Self::BrowserSession => "browserSession",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RendererSessionMode {
    Initialized,
    UninitializedFallback,
}

impl RendererSessionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Initialized => "initialized",
            Self::UninitializedFallback => "uninitializedFallback",
        }
    }
}

struct CdpEvaluation {
    values: Vec<Value>,
    transport: CdpTransport,
    session_mode: RendererSessionMode,
}

struct RendererTargetObservation {
    target: CdpTarget,
    probe: Value,
    transport: CdpTransport,
    session_mode: RendererSessionMode,
}

struct SessionEvaluation {
    values: Vec<Value>,
    mode: RendererSessionMode,
}

struct ThemeInstallation {
    value: Value,
    transport: CdpTransport,
    session_mode: RendererSessionMode,
    theme_chunks: usize,
    css_chunks: usize,
    art_chunks: usize,
    data_bytes: usize,
    elapsed_ms: u64,
    renderer_timings: Value,
}

struct ThemeSessionInstallation {
    value: Value,
    theme_chunks: usize,
    css_chunks: usize,
    art_chunks: usize,
    data_bytes: usize,
    elapsed_ms: u64,
    renderer_timings: Value,
}

struct CdpAttemptError {
    error: AppError,
    retry_safe: bool,
    navigation_epoch: Option<u64>,
}

impl CdpAttemptError {
    fn retry_safe(error: AppError) -> Self {
        Self {
            error,
            retry_safe: true,
            navigation_epoch: None,
        }
    }

    fn terminal(error: AppError) -> Self {
        let error = match error {
            AppError::Runtime(message) => AppError::RendererInstallTerminal(message),
            error => error,
        };
        Self {
            error,
            retry_safe: false,
            navigation_epoch: None,
        }
    }

    fn with_navigation_epoch(mut self, navigation_epoch: Option<u64>) -> Self {
        self.navigation_epoch = navigation_epoch;
        self
    }
}

pub struct CdpProgress {
    pub level: &'static str,
    pub event: &'static str,
    pub message: String,
    pub data: Value,
}

pub type CdpProgressSink<'a> = &'a (dyn Fn(CdpProgress) + Sync);

#[derive(Debug, Clone, Copy)]
pub struct CdpApplyOutcome {
    pub applied_targets: usize,
    pub direct_targets: usize,
    pub browser_session_targets: usize,
    pub initialized_session_targets: usize,
    pub uninitialized_fallback_targets: usize,
    pub staged_transfer_targets: usize,
    pub transferred_chunks: usize,
    pub transferred_bytes: usize,
    pub install_elapsed_ms: u64,
}

impl CdpSession {
    async fn connect_page(target: &CdpTarget, port: u16) -> AppResult<Self> {
        let url = validated_page_url(target, port)?;
        Self::connect_url(url, format!("page/{}", target.id)).await
    }

    async fn connect_browser(
        client: &Client,
        port: u16,
        expected_browser_id: &str,
    ) -> AppResult<Self> {
        let version: VersionResponse = fetch_json(client, port, "/json/version").await?;
        let url = validated_debugger_url(&version.web_socket_debugger_url, port, "browser")?;
        let browser_id = url
            .path()
            .strip_prefix("/devtools/browser/")
            .filter(|value| valid_identifier(value))
            .ok_or_else(|| AppError::Runtime("CDP 浏览器身份路径无效".into()))?
            .to_string();
        if browser_id != expected_browser_id {
            return Err(AppError::Runtime(format!(
                "CDP 浏览器身份已改变：{expected_browser_id} -> {browser_id}"
            )));
        }
        let mut session = Self::connect_url(url, format!("browser/{browser_id}")).await?;
        session
            .command(None, "Browser.getVersion", json!({}), COMMAND_TIMEOUT)
            .await?;
        Ok(session)
    }

    async fn connect_url(url: Url, endpoint: String) -> AppResult<Self> {
        let (stream, _) = timeout(
            CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(url.as_str()),
        )
        .await
        .map_err(|_| AppError::Runtime("CDP WebSocket 连接超时".into()))?
        .map_err(|error| AppError::Runtime(format!("CDP WebSocket 连接失败：{error}")))?;
        Ok(Self {
            endpoint,
            stream,
            next_id: 1,
        })
    }

    async fn evaluate(
        &mut self,
        session_id: Option<&str>,
        expression: &str,
        wait: Duration,
    ) -> AppResult<Value> {
        let result = self
            .command(
                session_id,
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "awaitPromise": true,
                    "returnByValue": true,
                    "userGesture": false,
                }),
                wait,
            )
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            return Err(AppError::Runtime(format!("渲染器执行失败：{exception}")));
        }
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn verify_stable_renderer(&mut self, session_id: Option<&str>) -> AppResult<Value> {
        let first = self
            .evaluate(session_id, PROBE_EXPRESSION, RENDERER_PROBE_TIMEOUT)
            .await?;
        if !probe_is_codex(Some(&first)) || !renderer_probe_is_presentable(Some(&first)) {
            return Err(AppError::Runtime(
                "渲染目标当前不可见、布局未完成或不是 Codex 主界面".into(),
            ));
        }
        let first_epoch = renderer_navigation_epoch(&first)
            .ok_or_else(|| AppError::Runtime("渲染目标缺少导航生命周期标识".into()))?;
        sleep(TARGET_STABILITY_DELAY).await;
        let second = self
            .evaluate(session_id, PROBE_EXPRESSION, RENDERER_PROBE_TIMEOUT)
            .await?;
        let second_epoch = renderer_navigation_epoch(&second);
        if !probe_is_codex(Some(&second))
            || !renderer_probe_is_presentable(Some(&second))
            || second_epoch != Some(first_epoch)
        {
            return Err(AppError::Runtime(
                "渲染目标在稳定性确认期间发生导航或失去可见性".into(),
            ));
        }
        Ok(second)
    }

    async fn install_theme(
        &mut self,
        session_id: Option<&str>,
        target_id: &str,
        payload: &RendererPayload,
        progress: Option<CdpProgressSink<'_>>,
    ) -> Result<ThemeSessionInstallation, CdpAttemptError> {
        let started = Instant::now();
        let plan = ThemeTransferPlan::new(payload);
        emit_progress(
            progress,
            "info",
            "cdp_theme_stage_started",
            "Renderer theme staging started",
            json!({
                "targetId": target_id,
                "endpoint": self.endpoint,
                "themeKey": payload.theme_key(),
                "engineBytes": payload.engine().len(),
                "themeBytes": plan.theme_bytes(),
                "themeSha256": plan.theme_sha256(),
                "cssBytes": plan.css_bytes(),
                "cssSha256": plan.css_sha256(),
                "artBytes": plan.art_bytes(),
                "artSha256": plan.art_sha256(),
                "themeChunks": plan.theme_chunk_count(),
                "cssChunks": plan.css_chunk_count(),
                "artChunks": plan.art_chunk_count(),
            }),
        );

        let engine = self
            .evaluate(session_id, payload.engine(), TRANSFER_COMMAND_TIMEOUT)
            .await
            .map_err(|error| {
                CdpAttemptError::retry_safe(AppError::Runtime(format!(
                    "CDP 主题引擎初始化失败：{error}"
                )))
            })?;
        if engine.get("ready").and_then(Value::as_bool) != Some(true)
            || engine.get("engineVersion").and_then(Value::as_u64) != Some(1)
        {
            return Err(CdpAttemptError::retry_safe(AppError::Runtime(
                "CDP 主题引擎未返回有效确认".into(),
            )));
        }
        emit_progress(
            progress,
            "info",
            "cdp_theme_engine_ready",
            "Renderer theme engine is ready",
            json!({
                "targetId": target_id,
                "endpoint": self.endpoint,
                "elapsedMs": elapsed_ms(started),
                "reused": engine.get("reused").and_then(Value::as_bool),
            }),
        );

        let initialize = plan.initialize_expression();
        let initialized = match self
            .evaluate(session_id, &initialize, TRANSFER_COMMAND_TIMEOUT)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                self.cleanup_theme_transfer(session_id, &plan).await;
                return Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
                    "CDP 主题数据通道初始化失败：{error}"
                ))));
            }
        };
        if !theme_stage_acknowledged(&initialized, plan.token(), "initialized") {
            self.cleanup_theme_transfer(session_id, &plan).await;
            return Err(CdpAttemptError::retry_safe(AppError::Runtime(
                "CDP 主题数据通道初始化未被渲染器确认".into(),
            )));
        }

        for index in 0..plan.theme_chunk_count() {
            let expression = plan.theme_chunk_expression(index);
            let acknowledgement = match self
                .evaluate(session_id, &expression, TRANSFER_COMMAND_TIMEOUT)
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    self.cleanup_theme_transfer(session_id, &plan).await;
                    return Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
                        "CDP 主题元数据分片传输失败：分片 {}/{}：{error}",
                        index + 1,
                        plan.theme_chunk_count(),
                    ))));
                }
            };
            let expected_bytes = plan.theme_received_bytes_after(index);
            if !theme_data_chunk_acknowledged(
                &acknowledgement,
                plan.token(),
                "themeReceiving",
                index + 1,
                expected_bytes,
            ) {
                self.cleanup_theme_transfer(session_id, &plan).await;
                return Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
                    "CDP 主题元数据分片确认无效：分片 {}/{}",
                    index + 1,
                    plan.theme_chunk_count(),
                ))));
            }
        }
        emit_progress(
            progress,
            "info",
            "cdp_theme_metadata_transferred",
            "Renderer theme metadata transferred",
            json!({
                "targetId": target_id,
                "endpoint": self.endpoint,
                "themeBytes": plan.theme_bytes(),
                "themeSha256": plan.theme_sha256(),
                "themeChunks": plan.theme_chunk_count(),
                "elapsedMs": elapsed_ms(started),
            }),
        );

        for index in 0..plan.css_chunk_count() {
            let expression = plan.css_chunk_expression(index);
            let acknowledgement = match self
                .evaluate(session_id, &expression, TRANSFER_COMMAND_TIMEOUT)
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    self.cleanup_theme_transfer(session_id, &plan).await;
                    return Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
                        "CDP 主题 CSS 分片传输失败：分片 {}/{}：{error}",
                        index + 1,
                        plan.css_chunk_count(),
                    ))));
                }
            };
            let expected_bytes = plan.css_received_bytes_after(index);
            if !theme_data_chunk_acknowledged(
                &acknowledgement,
                plan.token(),
                "cssReceiving",
                index + 1,
                expected_bytes,
            ) {
                self.cleanup_theme_transfer(session_id, &plan).await;
                return Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
                    "CDP 主题 CSS 分片确认无效：分片 {}/{}",
                    index + 1,
                    plan.css_chunk_count(),
                ))));
            }
        }
        emit_progress(
            progress,
            "info",
            "cdp_theme_css_transferred",
            "Renderer theme CSS transferred",
            json!({
                "targetId": target_id,
                "endpoint": self.endpoint,
                "cssBytes": plan.css_bytes(),
                "cssSha256": plan.css_sha256(),
                "cssChunks": plan.css_chunk_count(),
                "elapsedMs": elapsed_ms(started),
            }),
        );

        for index in 0..plan.art_chunk_count() {
            let expression = plan.art_chunk_expression(index);
            let acknowledgement = match self
                .evaluate(session_id, &expression, TRANSFER_COMMAND_TIMEOUT)
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    self.cleanup_theme_transfer(session_id, &plan).await;
                    return Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
                        "CDP 背景图分片传输失败：分片 {}/{}：{error}",
                        index + 1,
                        plan.art_chunk_count(),
                    ))));
                }
            };
            let expected_bytes = plan.art_received_bytes_after(index);
            if !theme_data_chunk_acknowledged(
                &acknowledgement,
                plan.token(),
                "artReceiving",
                index + 1,
                expected_bytes,
            ) {
                self.cleanup_theme_transfer(session_id, &plan).await;
                return Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
                    "CDP 背景图分片确认无效：分片 {}/{}",
                    index + 1,
                    plan.art_chunk_count(),
                ))));
            }
            if should_log_chunk(index + 1, plan.art_chunk_count()) {
                emit_progress(
                    progress,
                    "info",
                    "cdp_theme_art_progress",
                    "Renderer theme art transfer progressed",
                    json!({
                        "targetId": target_id,
                        "endpoint": self.endpoint,
                        "receivedChunks": index + 1,
                        "totalChunks": plan.art_chunk_count(),
                        "receivedBytes": expected_bytes,
                        "totalBytes": plan.art_bytes(),
                        "elapsedMs": elapsed_ms(started),
                    }),
                );
            }
        }

        let start_expression = plan.start_expression();
        let queued = match self
            .evaluate(session_id, &start_expression, INSTALL_START_TIMEOUT)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                emit_progress(
                    progress,
                    "error",
                    "cdp_theme_install_failed",
                    "Renderer theme installation start was not acknowledged",
                    json!({
                        "targetId": target_id,
                        "endpoint": self.endpoint,
                        "phase": "queueing",
                        "elapsedMs": elapsed_ms(started),
                        "error": error.to_string(),
                        "retrySuppressed": true,
                    }),
                );
                return Err(CdpAttemptError::terminal(AppError::Runtime(format!(
                    "CDP 主题安装启动状态未知；为避免重复注入，不再回退重试：{error}"
                ))));
            }
        };
        if !theme_stage_acknowledged(&queued, plan.token(), "queued") {
            emit_install_failure(
                progress,
                target_id,
                &self.endpoint,
                "queueing",
                elapsed_ms(started),
                "renderer returned an invalid queue acknowledgement",
            );
            return Err(CdpAttemptError::terminal(AppError::Runtime(
                "CDP 主题安装任务未被渲染器确认；为避免重复注入，不再回退重试".into(),
            )));
        }
        emit_progress(
            progress,
            "info",
            "cdp_theme_install_queued",
            "Renderer theme installation queued",
            json!({
                "targetId": target_id,
                "endpoint": self.endpoint,
                "themeChunks": plan.theme_chunk_count(),
                "cssChunks": plan.css_chunk_count(),
                "artChunks": plan.art_chunk_count(),
                "transferredBytes": payload.data_bytes(),
                "elapsedMs": elapsed_ms(started),
            }),
        );

        let status_expression = plan.status_expression();
        let deadline = Instant::now() + INSTALL_DEADLINE;
        let mut last_phase = "queued".to_string();
        loop {
            if Instant::now() >= deadline {
                emit_install_failure(
                    progress,
                    target_id,
                    &self.endpoint,
                    &last_phase,
                    elapsed_ms(started),
                    "installation deadline exceeded",
                );
                return Err(CdpAttemptError::terminal(AppError::Runtime(format!(
                    "CDP 主题安装在阶段 {last_phase} 超时；渲染页可能仍在执行，已停止回退重试"
                ))));
            }
            sleep(INSTALL_POLL_INTERVAL).await;
            let status = match self
                .evaluate(session_id, &status_expression, INSTALL_POLL_TIMEOUT)
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    emit_install_failure(
                        progress,
                        target_id,
                        &self.endpoint,
                        &last_phase,
                        elapsed_ms(started),
                        &error.to_string(),
                    );
                    return Err(CdpAttemptError::terminal(AppError::Runtime(format!(
                        "CDP 主题安装在阶段 {last_phase} 后失去响应；为避免阻塞扩散，已停止回退重试：{error}"
                    ))));
                }
            };
            if status.get("accepted").and_then(Value::as_bool) != Some(true) {
                emit_install_failure(
                    progress,
                    target_id,
                    &self.endpoint,
                    &last_phase,
                    elapsed_ms(started),
                    "renderer installation state is missing",
                );
                return Err(CdpAttemptError::terminal(AppError::Runtime(
                    "CDP 主题安装状态已丢失；已停止回退重试".into(),
                )));
            }
            let phase = status
                .get("phase")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if phase != last_phase {
                last_phase = phase.to_string();
                emit_progress(
                    progress,
                    "info",
                    "cdp_theme_install_phase",
                    "Renderer theme installation phase changed",
                    json!({
                        "targetId": target_id,
                        "endpoint": self.endpoint,
                        "phase": phase,
                        "elapsedMs": elapsed_ms(started),
                        "rendererTimings": status.get("timings").cloned().unwrap_or(Value::Null),
                    }),
                );
            }
            match phase {
                "installed" => {
                    let value = status.get("result").cloned().unwrap_or(Value::Null);
                    let renderer_timings = status.get("timings").cloned().unwrap_or(Value::Null);
                    self.cleanup_theme_transfer(session_id, &plan).await;
                    let elapsed_ms = elapsed_ms(started);
                    emit_progress(
                        progress,
                        "info",
                        "cdp_theme_install_completed",
                        "Renderer theme installation completed",
                        json!({
                            "targetId": target_id,
                            "endpoint": self.endpoint,
                            "elapsedMs": elapsed_ms,
                            "rendererTimings": renderer_timings,
                            "themeChunks": plan.theme_chunk_count(),
                            "cssChunks": plan.css_chunk_count(),
                            "artChunks": plan.art_chunk_count(),
                            "transferredBytes": payload.data_bytes(),
                            "presentation": renderer_presentation(&value),
                        }),
                    );
                    return Ok(ThemeSessionInstallation {
                        value,
                        theme_chunks: plan.theme_chunk_count(),
                        css_chunks: plan.css_chunk_count(),
                        art_chunks: plan.art_chunk_count(),
                        data_bytes: payload.data_bytes(),
                        elapsed_ms,
                        renderer_timings,
                    });
                }
                "failed" => {
                    let error = status
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown renderer failure");
                    let failed_phase = status
                        .get("failedPhase")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let timings = status.get("timings").cloned().unwrap_or(Value::Null);
                    self.cleanup_theme_transfer(session_id, &plan).await;
                    emit_progress(
                        progress,
                        "error",
                        "cdp_theme_install_failed",
                        "Renderer theme installation failed",
                        json!({
                            "targetId": target_id,
                            "endpoint": self.endpoint,
                            "phase": failed_phase,
                            "elapsedMs": elapsed_ms(started),
                            "rendererTimings": timings,
                            "error": error,
                            "retrySuppressed": true,
                        }),
                    );
                    return Err(CdpAttemptError::terminal(AppError::Runtime(format!(
                        "CDP 主题安装在渲染器阶段 {failed_phase} 失败：{error}"
                    ))));
                }
                _ => {}
            }
        }
    }

    async fn cleanup_theme_transfer(&mut self, session_id: Option<&str>, plan: &ThemeTransferPlan) {
        let cleanup = plan.cleanup_expression();
        let _ = self
            .evaluate(session_id, &cleanup, TRANSFER_CLEANUP_TIMEOUT)
            .await;
    }

    async fn initialize_renderer(&mut self, session_id: Option<&str>) -> AppResult<()> {
        for method in RENDERER_SESSION_DOMAINS {
            self.command(session_id, method, json!({}), COMMAND_TIMEOUT)
                .await
                .map_err(|error| {
                    AppError::Runtime(format!("CDP 渲染会话初始化失败（{method}）：{error}"))
                })?;
        }
        Ok(())
    }

    async fn command(
        &mut self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        wait: Duration,
    ) -> AppResult<Value> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let request = command_request(id, session_id, method, params).to_string();
        let request_bytes = request.len();
        let deadline = Instant::now() + wait;
        timeout_at(deadline, self.stream.send(Message::Text(request.into())))
            .await
            .map_err(|_| {
                AppError::Runtime(format!(
                    "CDP 命令发送超时：{method}，目标 {}，载荷 {request_bytes} bytes",
                    self.endpoint
                ))
            })?
            .map_err(|error| AppError::Runtime(format!("发送 CDP 命令失败：{error}")))?;

        let mut received_frames = 0_u64;
        let mut event_frames = 0_u64;
        let mut last_event_method = None;
        let response = loop {
            let next = timeout_at(deadline, self.stream.next())
                .await
                .map_err(|_| {
                    let last_event = last_event_method.as_deref().unwrap_or("none");
                    AppError::Runtime(format!(
                        "CDP 命令等待超时：{method}，目标 {}，载荷 {request_bytes} bytes，已接收 {received_frames} 帧（事件 {event_frames}），最近事件 {last_event}",
                        self.endpoint,
                    ))
                })?;
            let message = next
                .ok_or_else(|| AppError::Runtime("CDP WebSocket 已关闭".into()))?
                .map_err(|error| AppError::Runtime(format!("CDP WebSocket 错误：{error}")))?;
            received_frames = received_frames.saturating_add(1);
            let text = match message {
                Message::Text(text) => text,
                Message::Close(_) => {
                    return Err(AppError::Runtime("CDP WebSocket 提前关闭".into()))
                }
                _ => continue,
            };
            let parsed: Value = serde_json::from_str(text.as_str())?;
            let matching_session = match session_id {
                Some(expected) => parsed.get("sessionId").and_then(Value::as_str) == Some(expected),
                None => true,
            };
            if parsed.get("id").and_then(Value::as_u64) == Some(id) && matching_session {
                break parsed;
            }
            if let Some(event) = parsed.get("method").and_then(Value::as_str) {
                event_frames = event_frames.saturating_add(1);
                last_event_method = Some(event.chars().take(96).collect::<String>());
            }
        };
        if let Some(error) = response.get("error") {
            return Err(AppError::Runtime(format!(
                "CDP 命令被拒绝：{method}：{error}"
            )));
        }
        response
            .get("result")
            .cloned()
            .ok_or_else(|| AppError::Runtime(format!("CDP 响应缺少 result：{method}")))
    }

    async fn close(mut self) {
        let _ = self.stream.close(None).await;
    }
}

fn emit_progress(
    progress: Option<CdpProgressSink<'_>>,
    level: &'static str,
    event: &'static str,
    message: impl Into<String>,
    data: Value,
) {
    if let Some(progress) = progress {
        progress(CdpProgress {
            level,
            event,
            message: message.into(),
            data,
        });
    }
}

fn emit_install_failure(
    progress: Option<CdpProgressSink<'_>>,
    target_id: &str,
    endpoint: &str,
    phase: &str,
    elapsed_ms: u64,
    error: &str,
) {
    emit_progress(
        progress,
        "error",
        "cdp_theme_install_failed",
        "Renderer theme installation became unresponsive",
        json!({
            "targetId": target_id,
            "endpoint": endpoint,
            "phase": phase,
            "elapsedMs": elapsed_ms,
            "error": error,
            "retrySuppressed": true,
        }),
    );
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn theme_stage_acknowledged(value: &Value, token: &str, phase: &str) -> bool {
    value.get("accepted").and_then(Value::as_bool) == Some(true)
        && value.get("token").and_then(Value::as_str) == Some(token)
        && value.get("phase").and_then(Value::as_str) == Some(phase)
}

fn theme_data_chunk_acknowledged(
    value: &Value,
    token: &str,
    phase: &str,
    expected_chunks: usize,
    expected_bytes: usize,
) -> bool {
    theme_stage_acknowledged(value, token, phase)
        && value.get("receivedChunks").and_then(Value::as_u64)
            == u64::try_from(expected_chunks).ok()
        && value.get("receivedBytes").and_then(Value::as_u64) == u64::try_from(expected_bytes).ok()
}

fn should_log_chunk(completed: usize, total: usize) -> bool {
    if completed == 1 || completed == total {
        return true;
    }
    let interval = total.div_ceil(4).max(1);
    completed % interval == 0
}

fn command_request(id: u64, session_id: Option<&str>, method: &str, params: Value) -> Value {
    let mut request = json!({ "id": id, "method": method, "params": params });
    if let Some(session_id) = session_id {
        request["sessionId"] = Value::String(session_id.into());
    }
    request
}

pub fn build_client() -> AppResult<Client> {
    Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(AppError::Network)
}

pub async fn browser_identity(client: &Client, port: u16) -> AppResult<BrowserIdentity> {
    let version: VersionResponse = fetch_json(client, port, "/json/version").await?;
    let url = validated_debugger_url(&version.web_socket_debugger_url, port, "browser")?;
    let id = url
        .path()
        .strip_prefix("/devtools/browser/")
        .filter(|value| valid_identifier(value))
        .ok_or_else(|| AppError::Runtime("CDP 浏览器身份路径无效".into()))?;
    Ok(BrowserIdentity { id: id.into() })
}

pub async fn verified_targets(
    client: &Client,
    port: u16,
    expected_browser_id: &str,
) -> AppResult<Vec<CdpTarget>> {
    let identity = browser_identity(client, port).await?;
    if identity.id != expected_browser_id {
        return Err(AppError::Runtime(format!(
            "CDP 浏览器身份已改变：{} -> {}",
            expected_browser_id, identity.id
        )));
    }
    let targets: Vec<CdpTarget> = fetch_json(client, port, "/json/list").await?;
    let mut targets: Vec<CdpTarget> = targets
        .into_iter()
        .filter(|target| valid_page_target(target, port))
        .collect();
    // The upstream injector probes every app:// page. Prefer the primary page,
    // but retain initialRoute windows as fallback candidates because current
    // Codex builds can expose the responsive renderer under either target.
    targets.sort_by_key(target_priority);
    deduplicate_targets_by_id(&mut targets);
    Ok(targets)
}

/// Collects a bounded, content-free renderer fingerprint for persistent failure logs.
/// It intentionally excludes text, form values, storage, query strings and page titles.
pub async fn diagnostic_snapshot(
    client: &Client,
    port: u16,
    expected_browser_id: Option<&str>,
) -> Value {
    collect_diagnostic_snapshot(client, port, expected_browser_id).await
}

async fn collect_diagnostic_snapshot(
    client: &Client,
    port: u16,
    expected_browser_id: Option<&str>,
) -> Value {
    let identity = match browser_identity(client, port).await {
        Ok(identity) => identity,
        Err(error) => {
            return json!({
                "port": port,
                "stage": "browserIdentity",
                "error": error.to_string(),
            });
        }
    };
    let identity_matches = expected_browser_id
        .map(|expected| expected == identity.id)
        .unwrap_or(true);
    if !identity_matches {
        return json!({
            "port": port,
            "stage": "browserIdentity",
            "browserId": identity.id,
            "identityMatches": false,
        });
    }

    let targets: Vec<CdpTarget> = match fetch_json(client, port, "/json/list").await {
        Ok(targets) => targets,
        Err(error) => {
            return json!({
                "port": port,
                "stage": "targetList",
                "browserId": identity.id,
                "identityMatches": true,
                "error": error.to_string(),
            });
        }
    };
    let total_targets = targets.len();
    let page_targets = targets
        .iter()
        .filter(|target| target.target_type == "page")
        .count();
    let eligible_targets = targets
        .iter()
        .filter(|target| valid_page_target(target, port))
        .count();
    let mut reports = Vec::new();
    let mut probed_targets = 0;
    for target in targets.into_iter().take(DIAGNOSTIC_TARGET_LIMIT) {
        let eligible = valid_page_target(&target, port);
        let mut report = json!({
            "targetId": target.id,
            "targetType": target.target_type,
            "location": safe_target_location(&target.url),
            "eligible": eligible,
        });
        if eligible && probed_targets < DIAGNOSTIC_PROBE_LIMIT {
            probed_targets += 1;
            match timeout(
                DIAGNOSTIC_PROBE_TIMEOUT,
                evaluate_many(
                    client,
                    &target,
                    port,
                    identity.id.as_str(),
                    &[PROBE_EXPRESSION],
                ),
            )
            .await
            {
                Ok(Ok(evaluation)) => {
                    report["probe"] = evaluation.values.into_iter().next().unwrap_or(Value::Null);
                    report["probeTransport"] = Value::String(evaluation.transport.as_str().into());
                    report["probeSessionMode"] =
                        Value::String(evaluation.session_mode.as_str().into());
                }
                Ok(Err(error)) => report["probeError"] = Value::String(error.to_string()),
                Err(_) => {
                    report["probeTimeoutMs"] =
                        Value::from(DIAGNOSTIC_PROBE_TIMEOUT.as_millis() as u64)
                }
            }
        } else if eligible {
            report["probeSkipped"] = Value::String("diagnosticProbeLimit".into());
        }
        reports.push(report);
    }
    json!({
        "port": port,
        "stage": "rendererProbe",
        "browserId": identity.id,
        "identityMatches": true,
        "totalTargets": total_targets,
        "pageTargets": page_targets,
        "eligibleTargets": eligible_targets,
        "reportedTargets": reports.len(),
        "probedTargets": probed_targets,
        "targetLimit": DIAGNOSTIC_TARGET_LIMIT,
        "probeLimit": DIAGNOSTIC_PROBE_LIMIT,
        "probeTimeoutMs": DIAGNOSTIC_PROBE_TIMEOUT.as_millis(),
        "bounded": true,
        "targets": reports,
    })
}

async fn select_presentable_targets(
    client: &Client,
    port: u16,
    browser_id: &str,
    progress: Option<CdpProgressSink<'_>>,
) -> AppResult<Vec<CdpTarget>> {
    let targets = verified_targets(client, port, browser_id).await?;
    let total_targets = targets.len();
    let mut observations = Vec::new();
    let mut reports = Vec::with_capacity(total_targets);
    let mut last_error = None;

    for target in targets {
        match evaluate_many(client, &target, port, browser_id, &[PROBE_EXPRESSION]).await {
            Ok(evaluation) => {
                let probe = evaluation.values.into_iter().next().unwrap_or(Value::Null);
                reports.push(renderer_observation_report(
                    &target,
                    &probe,
                    Some(evaluation.transport),
                    Some(evaluation.session_mode),
                    None,
                ));
                if probe_is_codex(Some(&probe)) && renderer_probe_is_presentable(Some(&probe)) {
                    observations.push(RendererTargetObservation {
                        target,
                        probe,
                        transport: evaluation.transport,
                        session_mode: evaluation.session_mode,
                    });
                }
            }
            Err(error) => {
                last_error = Some(error.to_string());
                reports.push(renderer_observation_report(
                    &target,
                    &Value::Null,
                    None,
                    None,
                    Some(error.to_string()),
                ));
            }
        }
    }

    observations.sort_by(|left, right| {
        renderer_probe_score(&right.probe)
            .cmp(&renderer_probe_score(&left.probe))
            .then_with(|| target_priority(&left.target).cmp(&target_priority(&right.target)))
    });
    let selected = observations.first().map(|item| item.target.id.clone());
    let selected_transport = observations.first().map(|item| item.transport.as_str());
    let selected_session_mode = observations.first().map(|item| item.session_mode.as_str());
    if selected.is_some() {
        emit_progress(
            progress,
            "info",
            "cdp_renderer_selection",
            "Visible Codex renderer selection completed",
            json!({
                "totalTargets": total_targets,
                "presentableTargets": observations.len(),
                "selectedTargetId": selected,
                "selectedProbeTransport": selected_transport,
                "selectedProbeSessionMode": selected_session_mode,
                "targets": reports,
            }),
        );
    }

    let Some(selected) = observations.into_iter().next() else {
        return Err(AppError::Runtime(format!(
            "尚未发现可见且完成布局的 Codex 主渲染页：{}",
            last_error.unwrap_or_else(|| "目标仍处于隐藏或启动状态".into())
        )));
    };
    Ok(vec![selected.target])
}

pub async fn apply_to_verified_targets(
    client: &Client,
    port: u16,
    browser_id: &str,
    payload: &RendererPayload,
    progress: Option<CdpProgressSink<'_>>,
) -> AppResult<CdpApplyOutcome> {
    let targets = select_presentable_targets(client, port, browser_id, progress).await?;
    let mut applied = 0;
    let mut direct_targets = 0;
    let mut browser_session_targets = 0;
    let mut initialized_session_targets = 0;
    let mut uninitialized_fallback_targets = 0;
    let mut staged_transfer_targets = 0;
    let mut transferred_chunks: usize = 0;
    let mut transferred_bytes: usize = 0;
    let mut install_elapsed_ms: u64 = 0;
    let mut last_error = None;
    for target in targets {
        match install_theme_to_target(client, &target, port, browser_id, payload, progress).await {
            Ok(installation) if theme_install_result_is_confirmed(&installation.value) => {
                applied += 1;
                match installation.transport {
                    CdpTransport::DirectPage => direct_targets += 1,
                    CdpTransport::BrowserSession => browser_session_targets += 1,
                }
                match installation.session_mode {
                    RendererSessionMode::Initialized => initialized_session_targets += 1,
                    RendererSessionMode::UninitializedFallback => {
                        uninitialized_fallback_targets += 1
                    }
                }
                staged_transfer_targets += 1;
                transferred_chunks = transferred_chunks
                    .saturating_add(installation.theme_chunks)
                    .saturating_add(installation.css_chunks)
                    .saturating_add(installation.art_chunks);
                transferred_bytes = transferred_bytes.saturating_add(installation.data_bytes);
                install_elapsed_ms = install_elapsed_ms.saturating_add(installation.elapsed_ms);
                emit_progress(
                    progress,
                    "info",
                    "cdp_theme_target_completed",
                    "Theme installed on a verified Codex renderer",
                    json!({
                        "targetId": target.id,
                        "location": safe_target_location(&target.url),
                        "transport": installation.transport.as_str(),
                        "sessionMode": installation.session_mode.as_str(),
                        "themeChunks": installation.theme_chunks,
                        "cssChunks": installation.css_chunks,
                        "artChunks": installation.art_chunks,
                        "transferredBytes": installation.data_bytes,
                        "elapsedMs": installation.elapsed_ms,
                        "rendererTimings": installation.renderer_timings,
                        "presentation": renderer_presentation(&installation.value),
                    }),
                );
            }
            Ok(installation) => {
                emit_progress(
                    progress,
                    "warn",
                    "cdp_theme_visibility_failed",
                    "Theme DOM mounted but visible paint verification failed",
                    json!({
                        "targetId": target.id,
                        "location": safe_target_location(&target.url),
                        "presentation": renderer_presentation(&installation.value),
                        "retrySuppressed": false,
                    }),
                );
                last_error = Some("主题 DOM 已挂载，但可见窗口或实际样式未通过验收".to_string());
            }
            Err(error) if error.is_renderer_install_terminal() => return Err(error),
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if applied == 0 {
        return Err(AppError::Runtime(format!(
            "没有找到经过验证的 Codex 渲染页：{}",
            last_error.unwrap_or_else(|| "目标列表为空".into())
        )));
    }
    Ok(CdpApplyOutcome {
        applied_targets: applied,
        direct_targets,
        browser_session_targets,
        initialized_session_targets,
        uninitialized_fallback_targets,
        staged_transfer_targets,
        transferred_chunks,
        transferred_bytes,
        install_elapsed_ms,
    })
}

pub async fn ensure_theme_on_verified_targets(
    client: &Client,
    port: u16,
    browser_id: &str,
    theme_key: &str,
    payload: &RendererPayload,
    progress: Option<CdpProgressSink<'_>>,
) -> AppResult<usize> {
    let targets = verified_targets(client, port, browser_id).await?;
    let health = guarded_expression(&theme_health_expression(theme_key)?);
    let mut healthy = 0;
    let mut presentable = 0;
    let mut hidden_codex = 0;
    let mut target_transition = false;
    let mut last_error = None;
    for target in targets {
        match evaluate_many(client, &target, port, browser_id, &[&health]).await {
            Ok(evaluation)
                if probe_is_codex(evaluation.values.first())
                    && guarded_result_is_presentable(evaluation.values.first())
                    && action_result_is_true(evaluation.values.first()) =>
            {
                presentable += 1;
                healthy += 1;
                continue;
            }
            Ok(evaluation)
                if probe_is_codex(evaluation.values.first())
                    && guarded_result_is_presentable(evaluation.values.first()) =>
            {
                presentable += 1;
            }
            Ok(evaluation) if probe_is_codex(evaluation.values.first()) => {
                hidden_codex += 1;
                continue;
            }
            Ok(_) => {
                last_error = Some("页面尚未达到安全可注入状态".to_string());
                continue;
            }
            Err(error) => {
                last_error = Some(error.to_string());
                continue;
            }
        }

        match install_theme_to_target(client, &target, port, browser_id, payload, progress).await {
            Ok(installation) if theme_install_result_is_confirmed(&installation.value) => {
                healthy += 1
            }
            Ok(installation) => {
                emit_progress(
                    progress,
                    "warn",
                    "cdp_theme_visibility_failed",
                    "Theme DOM mounted but visible paint verification failed",
                    json!({
                        "targetId": target.id,
                        "location": safe_target_location(&target.url),
                        "presentation": renderer_presentation(&installation.value),
                        "retrySuppressed": false,
                    }),
                );
                last_error = Some("重注入后可见窗口或实际样式未通过验收".to_string());
            }
            Err(error) if error.is_renderer_install_terminal() && healthy == 0 => {
                return Err(error)
            }
            Err(error) if error.is_renderer_install_terminal() => {
                last_error = Some(error.to_string())
            }
            Err(error) if error.is_renderer_target_transition() => {
                target_transition = true;
                last_error = Some(error.to_string());
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }
    if watcher_wait_is_safe(presentable, hidden_codex, healthy, target_transition) {
        return Ok(0);
    }
    if presentable == 0 {
        return Err(AppError::Runtime(format!(
            "没有观察到可见或隐藏的 Codex 主题渲染页：{}",
            last_error.unwrap_or_else(|| "目标列表为空".into())
        )));
    }
    if healthy == 0 {
        return Err(AppError::Runtime(format!(
            "没有找到健康的 Codex 主题渲染页：{}",
            last_error.unwrap_or_else(|| "目标列表为空".into())
        )));
    }
    Ok(healthy)
}

pub async fn count_codex_targets(client: &Client, port: u16, browser_id: &str) -> AppResult<usize> {
    let targets = verified_targets(client, port, browser_id).await?;
    let guarded_probe = guarded_expression("true");
    let mut verified = 0;
    for target in targets {
        let evaluation =
            evaluate_many(client, &target, port, browser_id, &[&guarded_probe]).await?;
        if probe_is_codex(evaluation.values.first()) {
            verified += 1;
        }
    }
    Ok(verified)
}

pub async fn remove_from_verified_targets(
    client: &Client,
    port: u16,
    browser_id: &str,
) -> AppResult<usize> {
    let targets = verified_targets(client, port, browser_id).await?;
    let mut removed = 0;
    let guarded_remove = guarded_expression(REMOVE_EXPRESSION);
    for target in targets {
        let evaluation =
            evaluate_many(client, &target, port, browser_id, &[&guarded_remove]).await?;
        if probe_is_codex(evaluation.values.first()) {
            removed += 1;
        }
    }
    Ok(removed)
}

async fn fetch_json<T: DeserializeOwned>(
    client: &Client,
    port: u16,
    resource: &str,
) -> AppResult<T> {
    let response = client
        .get(format!("http://127.0.0.1:{port}{resource}"))
        .send()
        .await?
        .error_for_status()?;
    response.json().await.map_err(AppError::Network)
}

async fn install_theme_to_target(
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    payload: &RendererPayload,
    progress: Option<CdpProgressSink<'_>>,
) -> AppResult<ThemeInstallation> {
    let direct_error = match install_theme_direct(target, port, payload, progress).await {
        Ok(installation) => return Ok(installation),
        Err(error) if !error.retry_safe => {
            if let Some(reason) =
                terminal_retry_reason(client, target, port, browser_id, error.navigation_epoch)
                    .await
            {
                emit_progress(
                    progress,
                    "warn",
                    "cdp_theme_target_transition",
                    "Renderer changed after installation was queued; retry remains safe",
                    json!({
                        "targetId": target.id,
                        "location": safe_target_location(&target.url),
                        "reason": reason,
                        "retrySuppressed": false,
                        "previousError": error.error.to_string(),
                    }),
                );
                return Err(AppError::RendererTargetTransition(format!(
                    "安装期间发生 {reason}，等待新主页面后重试"
                )));
            }
            return Err(error.error);
        }
        Err(error) => error.error,
    };
    match install_theme_attached(client, target, port, browser_id, payload, progress).await {
        Ok(installation) => Ok(installation),
        Err(browser_error) if !browser_error.retry_safe => {
            if let Some(reason) = terminal_retry_reason(
                client,
                target,
                port,
                browser_id,
                browser_error.navigation_epoch,
            )
            .await
            {
                emit_progress(
                    progress,
                    "warn",
                    "cdp_theme_target_transition",
                    "Renderer changed after installation was queued; retry remains safe",
                    json!({
                        "targetId": target.id,
                        "location": safe_target_location(&target.url),
                        "reason": reason,
                        "retrySuppressed": false,
                        "previousError": browser_error.error.to_string(),
                    }),
                );
                Err(AppError::RendererTargetTransition(format!(
                    "安装期间发生 {reason}，等待新主页面后重试"
                )))
            } else {
                Err(browser_error.error)
            }
        }
        Err(browser_error) => Err(AppError::Runtime(format!(
            "CDP 页面直连与浏览器会话均未能开始安全安装；页面直连：{direct_error}；浏览器会话：{}",
            browser_error.error
        ))),
    }
}

async fn terminal_retry_reason(
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    navigation_epoch: Option<u64>,
) -> Option<&'static str> {
    let targets = verified_targets(client, port, browser_id).await.ok()?;
    let Some(current) = targets.into_iter().find(|item| item.id == target.id) else {
        return Some("targetReplaced");
    };
    let fresh_probe = timeout(Duration::from_secs(3), async {
        let mut session = CdpSession::connect_page(&current, port).await?;
        let initialized = session.initialize_renderer(None).await;
        if initialized.is_err() {
            session = CdpSession::connect_page(&current, port).await?;
        }
        let probe = session
            .evaluate(None, PROBE_EXPRESSION, Duration::from_secs(2))
            .await?;
        session.close().await;
        Ok::<Value, AppError>(probe)
    })
    .await
    .ok()?
    .ok()?;
    terminal_retry_reason_for_probe(navigation_epoch, &fresh_probe)
}

fn terminal_retry_reason_for_probe(
    navigation_epoch: Option<u64>,
    probe: &Value,
) -> Option<&'static str> {
    let current_epoch = renderer_navigation_epoch(probe);
    (probe_is_codex(Some(probe))
        && navigation_epoch.is_some()
        && current_epoch.is_some()
        && current_epoch != navigation_epoch)
        .then_some("targetNavigated")
}

async fn install_theme_direct(
    target: &CdpTarget,
    port: u16,
    payload: &RendererPayload,
    progress: Option<CdpProgressSink<'_>>,
) -> Result<ThemeInstallation, CdpAttemptError> {
    let initialized_error =
        match install_theme_direct_attempt(target, port, payload, progress, true).await {
            Ok(installation) => {
                return Ok(ThemeInstallation {
                    value: installation.value,
                    transport: CdpTransport::DirectPage,
                    session_mode: RendererSessionMode::Initialized,
                    theme_chunks: installation.theme_chunks,
                    css_chunks: installation.css_chunks,
                    art_chunks: installation.art_chunks,
                    data_bytes: installation.data_bytes,
                    elapsed_ms: installation.elapsed_ms,
                    renderer_timings: installation.renderer_timings,
                })
            }
            Err(error) if !error.retry_safe => return Err(error),
            Err(error) => error.error,
        };
    match install_theme_direct_attempt(target, port, payload, progress, false).await {
        Ok(installation) => Ok(ThemeInstallation {
            value: installation.value,
            transport: CdpTransport::DirectPage,
            session_mode: RendererSessionMode::UninitializedFallback,
            theme_chunks: installation.theme_chunks,
            css_chunks: installation.css_chunks,
            art_chunks: installation.art_chunks,
            data_bytes: installation.data_bytes,
            elapsed_ms: installation.elapsed_ms,
            renderer_timings: installation.renderer_timings,
        }),
        Err(error) if !error.retry_safe => Err(error),
        Err(error) => Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
            "页面初始化会话与无初始化回退均失败；初始化会话：{initialized_error}；无初始化回退：{}",
            error.error
        )))),
    }
}

async fn install_theme_direct_attempt(
    target: &CdpTarget,
    port: u16,
    payload: &RendererPayload,
    progress: Option<CdpProgressSink<'_>>,
    initialize: bool,
) -> Result<ThemeSessionInstallation, CdpAttemptError> {
    let mut session = CdpSession::connect_page(target, port)
        .await
        .map_err(CdpAttemptError::retry_safe)?;
    if initialize {
        session
            .initialize_renderer(None)
            .await
            .map_err(CdpAttemptError::retry_safe)?;
    }
    let stable_probe = session
        .verify_stable_renderer(None)
        .await
        .map_err(CdpAttemptError::retry_safe)?;
    let navigation_epoch = renderer_navigation_epoch(&stable_probe);
    let result = session
        .install_theme(None, &target.id, payload, progress)
        .await
        .map_err(|error| error.with_navigation_epoch(navigation_epoch));
    let should_close = match &result {
        Ok(_) => true,
        Err(error) => error.retry_safe,
    };
    if should_close {
        session.close().await;
    }
    result
}

async fn install_theme_attached(
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    payload: &RendererPayload,
    progress: Option<CdpProgressSink<'_>>,
) -> Result<ThemeInstallation, CdpAttemptError> {
    let initialized_error = match install_theme_attached_attempt(
        client, target, port, browser_id, payload, progress, true,
    )
    .await
    {
        Ok(installation) => {
            return Ok(ThemeInstallation {
                value: installation.value,
                transport: CdpTransport::BrowserSession,
                session_mode: RendererSessionMode::Initialized,
                theme_chunks: installation.theme_chunks,
                css_chunks: installation.css_chunks,
                art_chunks: installation.art_chunks,
                data_bytes: installation.data_bytes,
                elapsed_ms: installation.elapsed_ms,
                renderer_timings: installation.renderer_timings,
            })
        }
        Err(error) if !error.retry_safe => return Err(error),
        Err(error) => error.error,
    };
    match install_theme_attached_attempt(
        client, target, port, browser_id, payload, progress, false,
    )
    .await
    {
        Ok(installation) => Ok(ThemeInstallation {
            value: installation.value,
            transport: CdpTransport::BrowserSession,
            session_mode: RendererSessionMode::UninitializedFallback,
            theme_chunks: installation.theme_chunks,
            css_chunks: installation.css_chunks,
            art_chunks: installation.art_chunks,
            data_bytes: installation.data_bytes,
            elapsed_ms: installation.elapsed_ms,
            renderer_timings: installation.renderer_timings,
        }),
        Err(error) if !error.retry_safe => Err(error),
        Err(error) => Err(CdpAttemptError::retry_safe(AppError::Runtime(format!(
            "浏览器初始化会话与无初始化回退均失败；初始化会话：{initialized_error}；无初始化回退：{}",
            error.error
        )))),
    }
}

async fn install_theme_attached_attempt(
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    payload: &RendererPayload,
    progress: Option<CdpProgressSink<'_>>,
    initialize: bool,
) -> Result<ThemeSessionInstallation, CdpAttemptError> {
    let mut browser = CdpSession::connect_browser(client, port, browser_id)
        .await
        .map_err(CdpAttemptError::retry_safe)?;
    let attached = browser
        .command(
            None,
            "Target.attachToTarget",
            json!({ "targetId": target.id.as_str(), "flatten": true }),
            COMMAND_TIMEOUT,
        )
        .await
        .map_err(CdpAttemptError::retry_safe)?;
    let session_id = attached
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| valid_identifier(value))
        .ok_or_else(|| {
            CdpAttemptError::retry_safe(AppError::Runtime("CDP 浏览器附加会话身份无效".into()))
        })?
        .to_string();
    if initialize {
        browser
            .initialize_renderer(Some(session_id.as_str()))
            .await
            .map_err(CdpAttemptError::retry_safe)?;
    }
    let stable_probe = browser
        .verify_stable_renderer(Some(session_id.as_str()))
        .await
        .map_err(CdpAttemptError::retry_safe)?;
    let navigation_epoch = renderer_navigation_epoch(&stable_probe);
    let result = browser
        .install_theme(Some(session_id.as_str()), &target.id, payload, progress)
        .await
        .map_err(|error| error.with_navigation_epoch(navigation_epoch));
    let should_close = match &result {
        Ok(_) => true,
        Err(error) => error.retry_safe,
    };
    if should_close {
        let _ = browser
            .command(
                None,
                "Target.detachFromTarget",
                json!({ "sessionId": session_id }),
                COMMAND_TIMEOUT,
            )
            .await;
        browser.close().await;
    }
    result
}

async fn evaluate_many(
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    expressions: &[&str],
) -> AppResult<CdpEvaluation> {
    let direct_error = match evaluate_many_direct(target, port, expressions).await {
        Ok(evaluation) => {
            return Ok(CdpEvaluation {
                values: evaluation.values,
                transport: CdpTransport::DirectPage,
                session_mode: evaluation.mode,
            })
        }
        Err(error) => error,
    };
    match evaluate_many_attached(client, target, port, browser_id, expressions).await {
        Ok(evaluation) => Ok(CdpEvaluation {
            values: evaluation.values,
            transport: CdpTransport::BrowserSession,
            session_mode: evaluation.mode,
        }),
        Err(browser_error) => Err(AppError::Runtime(format!(
            "CDP 页面直连与浏览器会话均失败；页面直连：{direct_error}；浏览器会话：{browser_error}"
        ))),
    }
}

async fn evaluate_many_direct(
    target: &CdpTarget,
    port: u16,
    expressions: &[&str],
) -> AppResult<SessionEvaluation> {
    let initialized_error =
        match evaluate_many_direct_attempt(target, port, expressions, true).await {
            Ok(values) => {
                return Ok(SessionEvaluation {
                    values,
                    mode: RendererSessionMode::Initialized,
                })
            }
            Err(error) => error,
        };
    match evaluate_many_direct_attempt(target, port, expressions, false).await {
        Ok(values) => Ok(SessionEvaluation {
            values,
            mode: RendererSessionMode::UninitializedFallback,
        }),
        Err(fallback_error) => Err(AppError::Runtime(format!(
            "页面初始化会话与无初始化回退均失败；初始化会话：{initialized_error}；无初始化回退：{fallback_error}"
        ))),
    }
}

async fn evaluate_many_direct_attempt(
    target: &CdpTarget,
    port: u16,
    expressions: &[&str],
    initialize: bool,
) -> AppResult<Vec<Value>> {
    let mut session = CdpSession::connect_page(target, port).await?;
    if initialize {
        session.initialize_renderer(None).await?;
    }
    let probe = session
        .evaluate(None, PROBE_EXPRESSION, RENDERER_PROBE_TIMEOUT)
        .await?;
    if !probe_is_codex(Some(&probe)) {
        return Err(AppError::Runtime(
            "页面直连探针未确认可注入的 Codex 文档".into(),
        ));
    }
    let mut values = Vec::with_capacity(expressions.len());
    for expression in expressions {
        values.push(session.evaluate(None, expression, EVALUATE_TIMEOUT).await?);
    }
    session.close().await;
    Ok(values)
}

async fn evaluate_many_attached(
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    expressions: &[&str],
) -> AppResult<SessionEvaluation> {
    let initialized_error =
        match evaluate_many_attached_attempt(client, target, port, browser_id, expressions, true)
            .await
        {
            Ok(values) => {
                return Ok(SessionEvaluation {
                    values,
                    mode: RendererSessionMode::Initialized,
                })
            }
            Err(error) => error,
        };
    match evaluate_many_attached_attempt(client, target, port, browser_id, expressions, false).await
    {
        Ok(values) => Ok(SessionEvaluation {
            values,
            mode: RendererSessionMode::UninitializedFallback,
        }),
        Err(fallback_error) => Err(AppError::Runtime(format!(
            "浏览器初始化会话与无初始化回退均失败；初始化会话：{initialized_error}；无初始化回退：{fallback_error}"
        ))),
    }
}

async fn evaluate_many_attached_attempt(
    client: &Client,
    target: &CdpTarget,
    port: u16,
    browser_id: &str,
    expressions: &[&str],
    initialize: bool,
) -> AppResult<Vec<Value>> {
    let mut browser = CdpSession::connect_browser(client, port, browser_id).await?;
    let attached = browser
        .command(
            None,
            "Target.attachToTarget",
            json!({ "targetId": target.id.as_str(), "flatten": true }),
            COMMAND_TIMEOUT,
        )
        .await?;
    let session_id = attached
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|value| valid_identifier(value))
        .ok_or_else(|| AppError::Runtime("CDP 浏览器附加会话身份无效".into()))?
        .to_string();
    let result = async {
        if initialize {
            browser
                .initialize_renderer(Some(session_id.as_str()))
                .await?;
        }
        let probe = browser
            .evaluate(
                Some(session_id.as_str()),
                PROBE_EXPRESSION,
                RENDERER_PROBE_TIMEOUT,
            )
            .await?;
        if !probe_is_codex(Some(&probe)) {
            return Err(AppError::Runtime(
                "浏览器附加探针未确认可注入的 Codex 文档".into(),
            ));
        }
        let mut values = Vec::with_capacity(expressions.len());
        for expression in expressions {
            values.push(
                browser
                    .evaluate(Some(session_id.as_str()), expression, EVALUATE_TIMEOUT)
                    .await?,
            );
        }
        Ok(values)
    }
    .await;
    let _ = browser
        .command(
            None,
            "Target.detachFromTarget",
            json!({ "sessionId": session_id }),
            COMMAND_TIMEOUT,
        )
        .await;
    browser.close().await;
    result
}

fn probe_is_codex(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.get("codex"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn renderer_probe_is_presentable(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.pointer("/presentation/presentable"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn renderer_navigation_epoch(value: &Value) -> Option<u64> {
    value
        .pointer("/document/navigationEpoch")
        .and_then(Value::as_u64)
}

fn guarded_result_is_presentable(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.pointer("/presentation/presentable"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn watcher_wait_is_safe(
    presentable: usize,
    hidden_codex: usize,
    healthy: usize,
    target_transition: bool,
) -> bool {
    (presentable == 0 && hidden_codex > 0)
        || (presentable > 0 && healthy == 0 && target_transition)
}

fn renderer_probe_score(value: &Value) -> u64 {
    let focused = value
        .pointer("/presentation/hasFocus")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let marker_count = ["shell", "sidebar", "composer", "main"]
        .into_iter()
        .filter(|marker| {
            value
                .pointer(&format!("/markers/{marker}"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count() as u64;
    let element_count = value
        .pointer("/document/elementCount")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(100_000);
    (if focused { 1_000_000 } else { 0 }) + marker_count * 100_000 + element_count
}

fn renderer_presentation(value: &Value) -> Value {
    json!({
        "presentable": value.get("presentable").and_then(Value::as_bool),
        "paintVerified": value.get("paintVerified").and_then(Value::as_bool),
        "visibilityState": value.get("visibilityState").and_then(Value::as_str),
        "hasFocus": value.get("hasFocus").and_then(Value::as_bool),
        "viewportWidth": value.get("viewportWidth").and_then(Value::as_u64),
        "viewportHeight": value.get("viewportHeight").and_then(Value::as_u64),
        "surfaceReady": value.get("surfaceReady").and_then(Value::as_bool),
        "navigationEpoch": value.get("navigationEpoch").and_then(Value::as_u64),
    })
}

fn renderer_observation_report(
    target: &CdpTarget,
    probe: &Value,
    transport: Option<CdpTransport>,
    session_mode: Option<RendererSessionMode>,
    error: Option<String>,
) -> Value {
    json!({
        "targetId": target.id,
        "location": safe_target_location(&target.url),
        "codex": probe_is_codex(Some(probe)),
        "presentable": renderer_probe_is_presentable(Some(probe)),
        "visibilityState": probe.pointer("/presentation/visibilityState").and_then(Value::as_str),
        "hasFocus": probe.pointer("/presentation/hasFocus").and_then(Value::as_bool),
        "viewportWidth": probe.pointer("/presentation/viewportWidth").and_then(Value::as_u64),
        "viewportHeight": probe.pointer("/presentation/viewportHeight").and_then(Value::as_u64),
        "surfaceReady": probe.pointer("/presentation/surfaceReady").and_then(Value::as_bool),
        "readyState": probe.pointer("/document/readyState").and_then(Value::as_str),
        "elementCount": probe.pointer("/document/elementCount").and_then(Value::as_u64),
        "navigationEpoch": probe.pointer("/document/navigationEpoch").and_then(Value::as_u64),
        "probeTransport": transport.map(CdpTransport::as_str),
        "probeSessionMode": session_mode.map(RendererSessionMode::as_str),
        "error": error,
    })
}

fn action_result_is_true(value: Option<&Value>) -> bool {
    value
        .and_then(|value| value.get("result"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn theme_install_result_is_confirmed(result: &Value) -> bool {
    [
        "installed",
        "styleAttached",
        "rootTagged",
        "artAttached",
        "chromeAttached",
        "paintVerified",
        "presentable",
    ]
    .into_iter()
    .all(|field| result.get(field).and_then(Value::as_bool) == Some(true))
}

fn theme_health_expression(theme_key: &str) -> AppResult<String> {
    let key = serde_json::to_string(theme_key)?;
    Ok(format!(
        r#"(() => {{
  const state = window.__LUMADROBE_RUNTIME__;
  if (state?.themeKey !== {key} || typeof state?.ensure !== "function" ||
      typeof state?.status !== "function") return false;
  try {{
    state.ensure();
    const status = state.status();
    return Boolean(status?.installed && status?.styleAttached &&
      status?.rootTagged && status?.artAttached && status?.chromeAttached &&
      status?.paintVerified && status?.presentable);
  }} catch {{ return false; }}
}})()"#
    ))
}

fn guarded_expression(action: &str) -> String {
    format!(
        "(() => {{ const checked = ({PROBE_EXPRESSION}); if (!checked.codex) return checked; return {{ codex: true, presentation: checked.presentation, result: ({action}) }}; }})()"
    )
}

fn valid_page_target(target: &CdpTarget, port: u16) -> bool {
    let Ok(document_url) = Url::parse(&target.url) else {
        return false;
    };
    target.target_type == "page"
        && target.url.starts_with("app://")
        && document_url.scheme() == "app"
        && valid_identifier(&target.id)
        && validated_page_url(target, port).is_ok()
}

fn deduplicate_targets_by_id(targets: &mut Vec<CdpTarget>) {
    let mut seen = HashSet::new();
    targets.retain(|target| seen.insert(target.id.clone()));
}

fn target_priority(target: &CdpTarget) -> u8 {
    Url::parse(&target.url)
        .ok()
        .map(|url| {
            u8::from(
                url.query_pairs()
                    .any(|(key, _)| key.eq_ignore_ascii_case("initialRoute")),
            )
        })
        .unwrap_or(1)
}

fn safe_target_location(raw: &str) -> Value {
    match Url::parse(raw) {
        Ok(url) => json!({
            "scheme": url.scheme(),
            "host": url.host_str().map(|value| value.chars().take(80).collect::<String>()),
            "path": url.path().chars().take(160).collect::<String>(),
            "initialRoute": url.query_pairs().any(|(key, _)| key.eq_ignore_ascii_case("initialRoute")),
        }),
        Err(_) => json!({ "scheme": "invalid" }),
    }
}

fn validated_page_url(target: &CdpTarget, port: u16) -> AppResult<Url> {
    let url = validated_debugger_url(&target.web_socket_debugger_url, port, "page")?;
    if url.path() != format!("/devtools/page/{}", target.id) {
        return Err(AppError::Runtime(
            "CDP 页面身份与 WebSocket 路径不一致".into(),
        ));
    }
    Ok(url)
}

fn validated_debugger_url(value: &str, port: u16, kind: &str) -> AppResult<Url> {
    let url = Url::parse(value)
        .map_err(|error| AppError::Runtime(format!("CDP WebSocket URL 无效：{error}")))?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1"));
    let prefix = format!("/devtools/{kind}/");
    let id = url.path().strip_prefix(&prefix).unwrap_or_default();
    if url.scheme() != "ws"
        || !loopback
        || url.port() != Some(port)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !valid_identifier(id)
    {
        return Err(AppError::Runtime(
            "拒绝连接不符合回环端点规则的 CDP WebSocket".into(),
        ));
    }
    Ok(url)
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> CdpTarget {
        CdpTarget {
            id: "page-1".into(),
            target_type: "page".into(),
            url: "app://codex/home".into(),
            web_socket_debugger_url: url.into(),
        }
    }

    #[test]
    fn accepts_only_exact_loopback_page_urls() {
        assert!(valid_page_target(
            &target("ws://127.0.0.1:9341/devtools/page/page-1"),
            9341
        ));
        assert!(!valid_page_target(
            &target("ws://example.com:9341/devtools/page/page-1"),
            9341
        ));
        assert!(!valid_page_target(
            &target("ws://127.0.0.1:9342/devtools/page/page-1"),
            9341
        ));
        assert!(!valid_page_target(
            &target("ws://127.0.0.1:9341/devtools/page/other"),
            9341
        ));
        assert!(!valid_page_target(
            &target("ws://127.0.0.1:9341/devtools/page/page-1?token=x"),
            9341
        ));
    }

    #[test]
    fn rejects_non_app_targets_and_unsafe_ids() {
        let mut item = target("ws://127.0.0.1:9341/devtools/page/page-1");
        item.url = "https://example.com".into();
        assert!(!valid_page_target(&item, 9341));
        item.url = "app:settings".into();
        assert!(!valid_page_target(&item, 9341));
        item.url = "app:index.html".into();
        assert!(!valid_page_target(&item, 9341));
        item.url = "app://codex/home".into();
        item.id = "../browser".into();
        assert!(!valid_page_target(&item, 9341));
    }

    #[test]
    fn keeps_initial_route_targets_as_low_priority_fallbacks() {
        let mut item = target("ws://127.0.0.1:9341/devtools/page/page-1");
        item.url = "app://-/index.html?initialRoute=settings".into();
        assert!(valid_page_target(&item, 9341));
        assert_eq!(target_priority(&item), 1);

        item.url = "app://-/index.html?INITIALROUTE=settings".into();
        assert!(valid_page_target(&item, 9341));
        assert_eq!(target_priority(&item), 1);

        item.url = "app://-/index.html?route=settings".into();
        assert!(valid_page_target(&item, 9341));
        assert_eq!(target_priority(&item), 0);
    }

    #[test]
    fn deduplicates_repeated_page_entries_before_injection() {
        let first = target("ws://127.0.0.1:9341/devtools/page/page-1");
        let mut second = target("ws://127.0.0.1:9341/devtools/page/page-2");
        second.id = "page-2".into();
        let mut targets = vec![first.clone(), first, second];
        deduplicate_targets_by_id(&mut targets);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].id, "page-1");
        assert_eq!(targets[1].id, "page-2");
    }

    #[test]
    fn flattened_commands_carry_the_attached_session_id() {
        let request = command_request(
            7,
            Some("session-1"),
            "Runtime.evaluate",
            json!({ "expression": "true" }),
        );
        assert_eq!(request["id"], 7);
        assert_eq!(request["sessionId"], "session-1");
        assert_eq!(request["method"], "Runtime.evaluate");
        assert_eq!(request["params"]["expression"], "true");

        let browser_request = command_request(8, None, "Browser.getVersion", json!({}));
        assert!(browser_request.get("sessionId").is_none());
    }

    #[test]
    fn renderer_sessions_initialize_the_domains_used_by_the_upstream_injector() {
        assert_eq!(RENDERER_SESSION_DOMAINS, ["Runtime.enable", "Page.enable"]);
        assert_eq!(COMMAND_TIMEOUT, Duration::from_secs(10));
        assert_eq!(RENDERER_PROBE_TIMEOUT, Duration::from_secs(10));
    }

    #[test]
    fn renderer_session_modes_distinguish_compatibility_fallbacks() {
        assert_eq!(RendererSessionMode::Initialized.as_str(), "initialized");
        assert_eq!(
            RendererSessionMode::UninitializedFallback.as_str(),
            "uninitializedFallback"
        );
    }

    #[test]
    fn queued_install_failures_are_typed_as_terminal() {
        let error = CdpAttemptError::terminal(AppError::Runtime("renderer stalled".into()));
        assert!(!error.retry_safe);
        assert!(error.error.is_renderer_install_terminal());
    }

    #[test]
    fn renderer_transitions_remain_distinct_from_terminal_failures() {
        let error = AppError::RendererTargetTransition("target replaced".into());
        assert!(error.is_renderer_target_transition());
        assert!(!error.is_renderer_install_terminal());
    }

    #[test]
    fn terminal_retry_requires_an_observed_navigation_epoch_change() {
        let same_document = json!({
            "codex": true,
            "document": { "navigationEpoch": 42 },
        });
        assert_eq!(
            terminal_retry_reason_for_probe(Some(42), &same_document),
            None
        );
        assert_eq!(terminal_retry_reason_for_probe(None, &same_document), None);

        let navigated = json!({
            "codex": true,
            "document": { "navigationEpoch": 43 },
        });
        assert_eq!(
            terminal_retry_reason_for_probe(Some(42), &navigated),
            Some("targetNavigated")
        );

        let unverified = json!({
            "codex": false,
            "document": { "navigationEpoch": 43 },
        });
        assert_eq!(terminal_retry_reason_for_probe(Some(42), &unverified), None);
    }

    #[test]
    fn watcher_waits_only_for_observed_hidden_or_transitioning_renderers() {
        assert!(!watcher_wait_is_safe(0, 0, 0, false));
        assert!(watcher_wait_is_safe(0, 1, 0, false));
        assert!(watcher_wait_is_safe(1, 0, 0, true));
        assert!(!watcher_wait_is_safe(1, 0, 0, false));
        assert!(!watcher_wait_is_safe(1, 0, 1, true));
    }

    #[test]
    fn staged_theme_acknowledgements_require_token_phase_and_exact_progress() {
        let value = json!({
            "accepted": true,
            "token": "attempt-1",
            "phase": "artReceiving",
            "receivedChunks": 2,
            "receivedBytes": 1024,
        });
        assert!(theme_data_chunk_acknowledged(
            &value,
            "attempt-1",
            "artReceiving",
            2,
            1024,
        ));
        assert!(!theme_data_chunk_acknowledged(
            &value,
            "other-attempt",
            "artReceiving",
            2,
            1024,
        ));
        assert!(!theme_data_chunk_acknowledged(
            &value,
            "attempt-1",
            "cssReceiving",
            2,
            1024,
        ));
    }

    #[test]
    fn guarded_actions_run_only_after_the_codex_probe() {
        let guarded = guarded_expression("window.__testAction = true");
        let probe = guarded.find("if (!checked.codex)").unwrap();
        let action = guarded.find("window.__testAction").unwrap();
        assert!(probe < action);
        assert!(guarded.contains("presentation: checked.presentation"));
    }

    #[test]
    fn watcher_health_probe_is_small_and_theme_specific() {
        let expression = theme_health_expression("source:night@1.0.0").unwrap();
        assert!(expression.contains("source:night@1.0.0"));
        assert!(expression.contains("state.ensure()"));
        assert!(expression.contains("status?.paintVerified && status?.presentable"));
        assert!(!expression.contains("data:image"));
    }

    #[test]
    fn renderer_selection_requires_a_visible_laid_out_surface() {
        let visible = json!({
            "codex": true,
            "document": { "navigationEpoch": 42, "elementCount": 300 },
            "presentation": {
                "presentable": true,
                "visibilityState": "visible",
                "hasFocus": true,
                "viewportWidth": 1280,
                "viewportHeight": 800,
                "surfaceReady": true,
            },
            "markers": { "shell": true, "sidebar": true, "composer": false, "main": true },
        });
        assert!(renderer_probe_is_presentable(Some(&visible)));
        assert_eq!(renderer_navigation_epoch(&visible), Some(42));
        assert!(renderer_probe_score(&visible) > 1_000_000);

        let mut hidden = visible.clone();
        hidden["presentation"]["presentable"] = Value::Bool(false);
        hidden["presentation"]["visibilityState"] = Value::String("hidden".into());
        assert!(!renderer_probe_is_presentable(Some(&hidden)));
    }

    #[test]
    fn diagnostic_probe_is_bounded_and_content_free() {
        assert!(PROBE_EXPRESSION.contains("slice(0, 24)"));
        assert!(PROBE_EXPRESSION.contains("elementCount"));
        assert!(PROBE_EXPRESSION.contains("visibilityState === 'visible'"));
        assert!(PROBE_EXPRESSION.contains("navigationEpoch"));
        assert!(PROBE_EXPRESSION.contains("viewportWidth >= 320"));
        assert!(PROBE_EXPRESSION.contains("security.appProtocol && security.documentReady"));
        assert_eq!(DIAGNOSTIC_PROBE_TIMEOUT, Duration::from_secs(12));
        assert_eq!(DIAGNOSTIC_TARGET_LIMIT, 16);
        assert_eq!(DIAGNOSTIC_PROBE_LIMIT, 4);
        for forbidden in [
            "innerText",
            "textContent",
            "innerHTML",
            "outerHTML",
            "localStorage",
            "sessionStorage",
            "document.title",
            ".value",
        ] {
            assert!(
                !PROBE_EXPRESSION.contains(forbidden),
                "diagnostic probe must not capture {forbidden}"
            );
        }
    }

    #[test]
    fn theme_install_requires_observable_renderer_state() {
        let value = json!({
            "installed": true,
            "styleAttached": true,
            "rootTagged": true,
            "artAttached": true,
            "chromeAttached": true,
            "paintVerified": true,
            "presentable": true,
        });
        assert!(theme_install_result_is_confirmed(&value));
        let missing_style = json!({
            "installed": true,
            "styleAttached": false,
            "rootTagged": true,
            "artAttached": true,
            "chromeAttached": true,
            "paintVerified": true,
            "presentable": true,
        });
        assert!(!theme_install_result_is_confirmed(&missing_style));
        let missing_chrome = json!({
            "installed": true,
            "styleAttached": true,
            "rootTagged": true,
            "artAttached": true,
            "chromeAttached": false,
            "paintVerified": true,
            "presentable": true,
        });
        assert!(!theme_install_result_is_confirmed(&missing_chrome));
        let mut hidden = value;
        hidden["presentable"] = Value::Bool(false);
        assert!(!theme_install_result_is_confirmed(&hidden));
    }

    #[test]
    fn safe_target_location_drops_query_and_fragment() {
        let location = safe_target_location("app://codex/home?secret=value#private");
        assert_eq!(location["scheme"], "app");
        assert_eq!(location["host"], "codex");
        assert_eq!(location["path"], "/home");
        assert_eq!(location["initialRoute"], false);
        assert!(location.get("query").is_none());
        assert!(location.get("fragment").is_none());
    }
}
