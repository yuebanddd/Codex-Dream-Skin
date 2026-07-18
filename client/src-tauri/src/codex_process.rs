use crate::error::{AppError, AppResult};
#[cfg(target_os = "windows")]
use serde::Deserialize;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
use windows::core::HSTRING;
#[cfg(target_os = "windows")]
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED,
};
#[cfg(target_os = "windows")]
use windows::Win32::UI::Shell::{
    ApplicationActivationManager, IApplicationActivationManager, AO_NONE,
};

const EXPECTED_MAC_TEAM_ID: &str = "2DC432GLL2";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(target_os = "macos")]
const LEGACY_MACOS_JOB_LABEL: &str = "com.openai.codex-dream-skin-studio.app";

#[derive(Debug, Clone)]
pub struct CodexInstall {
    pub platform: String,
    pub executable: PathBuf,
    pub bundle_path: Option<PathBuf>,
    pub version: String,
    pub identity: String,
    pub app_user_model_id: Option<String>,
}

impl CodexInstall {
    pub fn discover() -> AppResult<Self> {
        #[cfg(target_os = "macos")]
        {
            discover_macos()
        }
        #[cfg(target_os = "windows")]
        {
            discover_windows()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(AppError::Runtime(
                "当前只支持 macOS 与 Windows Codex Desktop".into(),
            ))
        }
    }

    pub fn preferred_port(&self) -> u16 {
        if self.platform == "windows" {
            9335
        } else {
            9341
        }
    }

    pub fn launch_with_cdp(&self, port: u16) -> AppResult<Option<Child>> {
        #[cfg(target_os = "macos")]
        {
            clear_legacy_macos_jobs();
            let mut command = Command::new(&self.executable);
            command
                .arg("--remote-debugging-address=127.0.0.1")
                .arg(format!("--remote-debugging-port={port}"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            return command.spawn().map(Some).map_err(AppError::Io);
        }
        #[cfg(target_os = "windows")]
        {
            let app_user_model_id = self.app_user_model_id.as_deref().ok_or_else(|| {
                AppError::Runtime("已验证的 Codex Store 包缺少应用用户模型 ID".into())
            })?;
            activate_windows_store_app(app_user_model_id, &cdp_activation_arguments(port))?;
            return Ok(None);
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = port;
            Err(AppError::Runtime("当前平台不支持启动 Codex".into()))
        }
    }

    pub fn launch_normally(&self) -> AppResult<()> {
        #[cfg(target_os = "macos")]
        {
            clear_legacy_macos_jobs();
            let bundle = self
                .bundle_path
                .as_ref()
                .ok_or_else(|| AppError::Runtime("缺少已验证的 Codex 应用包路径".into()))?;
            let status = Command::new("/usr/bin/open")
                .arg("-na")
                .arg(bundle)
                .status()?;
            if !status.success() {
                return Err(AppError::Runtime("无法重新打开官方 Codex".into()));
            }
            Ok(())
        }
        #[cfg(target_os = "windows")]
        {
            let app_user_model_id = self.app_user_model_id.as_deref().ok_or_else(|| {
                AppError::Runtime("已验证的 Codex Store 包缺少应用用户模型 ID".into())
            })?;
            activate_windows_store_app(app_user_model_id, "")?;
            Ok(())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(AppError::Runtime("当前平台不支持启动 Codex".into()))
        }
    }

    pub fn is_running(&self) -> AppResult<bool> {
        #[cfg(target_os = "macos")]
        {
            Ok(!macos_codex_pids(&self.executable)?.is_empty())
        }
        #[cfg(target_os = "windows")]
        {
            run_windows_identity_script(
                &self.executable,
                r#"
$expected = [IO.Path]::GetFullPath($env:LUMADROBE_CODEX_EXE)
$running = Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
  $path = $_.ExecutablePath
  if (-not $path) {
    try { $path = (Get-Process -Id $_.ProcessId -ErrorAction Stop).Path } catch { $path = $null }
  }
  $path -and ([IO.Path]::GetFullPath($path) -ieq $expected)
}
if (@($running).Count -gt 0) { 'true' } else { 'false' }
"#,
            )
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Ok(false)
        }
    }

    pub fn saved_executable_is_running(platform: &str, executable: &str) -> AppResult<bool> {
        #[cfg(target_os = "macos")]
        {
            if platform != "macos" {
                return Ok(false);
            }
            Ok(!macos_codex_pids(Path::new(executable))?.is_empty())
        }
        #[cfg(target_os = "windows")]
        {
            if platform != "windows" {
                return Ok(false);
            }
            run_windows_identity_script(
                Path::new(executable),
                r#"
$expected = [IO.Path]::GetFullPath($env:LUMADROBE_CODEX_EXE)
$running = Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
  $path = $_.ExecutablePath
  if (-not $path) {
    try { $path = (Get-Process -Id $_.ProcessId -ErrorAction Stop).Path } catch { $path = $null }
  }
  $path -and ([IO.Path]::GetFullPath($path) -ieq $expected)
}
if (@($running).Count -gt 0) { 'true' } else { 'false' }
"#,
            )
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = (platform, executable);
            Ok(false)
        }
    }

    pub fn verify_listener_owner(&self, port: u16) -> AppResult<bool> {
        #[cfg(target_os = "macos")]
        {
            verify_macos_listener_owner(&self.executable, port)
        }
        #[cfg(target_os = "windows")]
        {
            let script = format!(
                r#"
$expected = [IO.Path]::GetFullPath($env:LUMADROBE_CODEX_EXE)
$listeners = @(Get-NetTCPConnection -State Listen -LocalPort {port} -ErrorAction Stop)
if ($listeners.Count -eq 0) {{ 'false'; exit 0 }}
foreach ($listener in $listeners) {{
  if ($listener.LocalAddress -notin @('127.0.0.1', '::1')) {{ 'false'; exit 0 }}
  $pidValue = [int]$listener.OwningProcess
  $matched = $false
  for ($depth = 0; $depth -lt 32 -and $pidValue -gt 1; $depth++) {{
    $process = Get-CimInstance Win32_Process -Filter "ProcessId = $pidValue" -ErrorAction SilentlyContinue
    if (-not $process) {{ break }}
    $path = $process.ExecutablePath
    if (-not $path) {{
      try {{ $path = (Get-Process -Id $pidValue -ErrorAction Stop).Path }} catch {{ $path = $null }}
    }}
    if ($path -and ([IO.Path]::GetFullPath($path) -ieq $expected)) {{ $matched = $true; break }}
    $pidValue = [int]$process.ParentProcessId
  }}
  if (-not $matched) {{ 'false'; exit 0 }}
}}
'true'
"#
            );
            run_windows_identity_script(&self.executable, &script)
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let _ = port;
            Ok(false)
        }
    }

    pub fn stop(&self, mut launched_child: Option<&mut Child>) -> AppResult<()> {
        #[cfg(target_os = "macos")]
        {
            clear_legacy_macos_jobs();
            let _ = Command::new("/usr/bin/osascript")
                .args(["-e", "tell application id \"com.openai.codex\" to quit"])
                .status();
            let deadline = Instant::now() + Duration::from_secs(15);
            while self.is_running()? && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(250));
            }
            if self.is_running()? {
                if let Some(child) = launched_child.as_mut() {
                    let _ = child.kill();
                    let _ = child.wait();
                    let force_deadline = Instant::now() + Duration::from_secs(5);
                    while self.is_running()? && Instant::now() < force_deadline {
                        thread::sleep(Duration::from_millis(250));
                    }
                }
            }
            if self.is_running()? {
                return Err(AppError::Runtime(
                    "Codex 未能安全退出；请手动退出后重试".into(),
                ));
            }
            if let Some(child) = launched_child.as_mut() {
                let _ = child.try_wait();
            }
            Ok(())
        }
        #[cfg(target_os = "windows")]
        {
            let result = run_windows_identity_script(
                &self.executable,
                r#"
$expected = [IO.Path]::GetFullPath($env:LUMADROBE_CODEX_EXE)
$matches = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
  $path = $_.ExecutablePath
  if (-not $path) {
    try { $path = (Get-Process -Id $_.ProcessId -ErrorAction Stop).Path } catch { $path = $null }
  }
  $path -and ([IO.Path]::GetFullPath($path) -ieq $expected)
})
foreach ($item in $matches) {
  try { (Get-Process -Id $item.ProcessId -ErrorAction Stop).CloseMainWindow() | Out-Null } catch {}
}
$deadline = [DateTime]::UtcNow.AddSeconds(12)
do {
  Start-Sleep -Milliseconds 250
  $alive = @($matches | Where-Object { Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue })
} while ($alive.Count -gt 0 -and [DateTime]::UtcNow -lt $deadline)
foreach ($item in $alive) {
  $path = $null
  try { $path = (Get-Process -Id $item.ProcessId -ErrorAction Stop).Path } catch {}
  if ($path -and ([IO.Path]::GetFullPath($path) -ieq $expected)) {
    Stop-Process -Id $item.ProcessId -Force -ErrorAction Stop
  }
}
'true'
"#,
            )?;
            if !result {
                return Err(AppError::Runtime("无法安全结束官方 Codex 进程".into()));
            }
            if let Some(child) = launched_child.as_mut() {
                if child.try_wait()?.is_none() {
                    let _ = child.kill();
                }
                let _ = child.wait();
            }
            Ok(())
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            Err(AppError::Runtime("当前平台不支持停止 Codex".into()))
        }
    }

    pub fn same_executable(&self, saved: &str) -> bool {
        let current = self.executable.to_string_lossy();
        if self.platform == "windows" {
            current.eq_ignore_ascii_case(saved)
        } else {
            current == saved
        }
    }
}

#[cfg(target_os = "macos")]
fn clear_legacy_macos_jobs() {
    let remove = |label: &str| {
        let _ = Command::new("/bin/launchctl")
            .args(["remove", label])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    };
    let uid = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|uid| !uid.is_empty());
    if let Some(uid) = uid {
        remove(&format!("gui/{uid}/{LEGACY_MACOS_JOB_LABEL}"));
    }
    remove(LEGACY_MACOS_JOB_LABEL);
}

#[cfg(target_os = "macos")]
fn discover_macos() -> AppResult<CodexInstall> {
    let mut candidates = Vec::new();
    if let Some(configured) = std::env::var_os("CODEX_APP_BUNDLE") {
        if !configured.is_empty() {
            candidates.push(PathBuf::from(configured));
        }
    }
    candidates.extend([
        PathBuf::from("/Applications/ChatGPT.app"),
        PathBuf::from("/Applications/Codex.app"),
    ]);
    if let Some(home) = std::env::var_os("HOME") {
        let applications = PathBuf::from(home).join("Applications");
        candidates.push(applications.join("ChatGPT.app"));
        candidates.push(applications.join("Codex.app"));
    }
    if let Ok(output) = Command::new("/usr/bin/mdfind")
        .arg("kMDItemCFBundleIdentifier == 'com.openai.codex'")
        .output()
    {
        if output.status.success() {
            candidates.extend(
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(PathBuf::from),
            );
        }
    }
    let mut unique_candidates = Vec::new();
    for candidate in candidates {
        if !unique_candidates.contains(&candidate) {
            unique_candidates.push(candidate);
        }
    }
    for bundle in unique_candidates {
        let plist = bundle.join("Contents/Info.plist");
        if !plist.is_file() {
            continue;
        }
        let Ok(identifier) = plutil_value(&plist, "CFBundleIdentifier") else {
            continue;
        };
        if identifier != "com.openai.codex" {
            continue;
        }
        if verify_macos_signature(&bundle).is_err() {
            continue;
        }
        let Ok(executable_name) = plutil_value(&plist, "CFBundleExecutable") else {
            continue;
        };
        let Ok(version) = plutil_value(&plist, "CFBundleShortVersionString") else {
            continue;
        };
        let executable = bundle.join("Contents/MacOS").join(executable_name);
        if !executable.is_file() {
            continue;
        }
        return Ok(CodexInstall {
            platform: "macos".into(),
            executable,
            bundle_path: Some(bundle),
            version,
            identity: format!("com.openai.codex/{EXPECTED_MAC_TEAM_ID}"),
            app_user_model_id: None,
        });
    }
    Err(AppError::Runtime(
        "找不到已签名的官方 Codex（com.openai.codex）".into(),
    ))
}

#[cfg(target_os = "macos")]
fn plutil_value(plist: &Path, key: &str) -> AppResult<String> {
    let output = Command::new("/usr/bin/plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(plist)
        .output()?;
    if !output.status.success() {
        return Err(AppError::Runtime(format!("无法读取 Codex 应用标识：{key}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(target_os = "macos")]
fn verify_macos_signature(bundle: &Path) -> AppResult<()> {
    let status = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .status()?;
    if !status.success() {
        return Err(AppError::Runtime("Codex 代码签名校验失败".into()));
    }
    let output = Command::new("/usr/bin/codesign")
        .args(["-dv", "--verbose=4"])
        .arg(bundle)
        .output()?;
    let details = String::from_utf8_lossy(&output.stderr);
    let team = details
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="))
        .unwrap_or_default();
    if team != EXPECTED_MAC_TEAM_ID {
        return Err(AppError::Runtime(format!(
            "Codex 签名团队不受信任：{}",
            if team.is_empty() { "missing" } else { team }
        )));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn macos_codex_pids(executable: &Path) -> AppResult<Vec<u32>> {
    let output = Command::new("/usr/sbin/lsof")
        .args(["-nP", "-d", "txt", "-Fpn"])
        .output()?;
    if !output.status.success() && output.stdout.is_empty() {
        return Err(AppError::Runtime(
            "无法读取 macOS 进程可执行文件映射".into(),
        ));
    }
    Ok(parse_macos_text_pids(
        &String::from_utf8_lossy(&output.stdout),
        &executable.to_string_lossy(),
    ))
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_text_pids(output: &str, expected: &str) -> Vec<u32> {
    let mut current_pid = None;
    let mut pids = Vec::new();
    for line in output.lines() {
        if let Some(value) = line.strip_prefix('p') {
            current_pid = value.parse::<u32>().ok();
        } else if let (Some(pid), Some(path)) = (current_pid, line.strip_prefix('n')) {
            let path = path.strip_suffix(" (deleted)").unwrap_or(path);
            if path == expected {
                pids.push(pid);
            }
        }
    }
    pids.sort_unstable();
    pids.dedup();
    pids
}

#[cfg(target_os = "macos")]
fn verify_macos_listener_owner(executable: &Path, port: u16) -> AppResult<bool> {
    let output = Command::new("/usr/sbin/lsof")
        .args([
            "-nP",
            "-a",
            &format!("-iTCP:{port}"),
            "-sTCP:LISTEN",
            "-Fpn",
        ])
        .output()?;
    if !output.status.success() && output.stdout.is_empty() {
        return Ok(false);
    }
    let mut current_pid = None;
    let mut listeners = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(value) = line.strip_prefix('p') {
            current_pid = value.parse::<u32>().ok();
        } else if let (Some(pid), Some(address)) = (current_pid, line.strip_prefix('n')) {
            listeners.push((pid, address.to_owned()));
        }
    }
    if listeners.is_empty()
        || listeners
            .iter()
            .any(|(_, address)| !macos_listener_is_loopback(address, port))
    {
        return Ok(false);
    }
    listeners.sort_by_key(|(pid, _)| *pid);
    listeners.dedup_by_key(|(pid, _)| *pid);
    let official_pids = macos_codex_pids(executable)?;
    for (pid, _) in listeners {
        if !macos_pid_descends_from_any(pid, &official_pids)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(any(target_os = "macos", test))]
fn macos_listener_is_loopback(address: &str, port: u16) -> bool {
    let address = address.strip_suffix(" (LISTEN)").unwrap_or(address);
    address == format!("127.0.0.1:{port}") || address == format!("[::1]:{port}")
}

#[cfg(target_os = "macos")]
fn macos_pid_descends_from_any(mut pid: u32, expected_pids: &[u32]) -> AppResult<bool> {
    for _ in 0..32 {
        if pid <= 1 {
            return Ok(false);
        }
        if expected_pids.contains(&pid) {
            return Ok(true);
        }
        let output = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "ppid="])
            .output()?;
        let parent = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u32>()
            .unwrap_or_default();
        if parent == 0 || parent == pid {
            return Ok(false);
        }
        pid = parent;
    }
    Ok(false)
}

#[cfg(target_os = "windows")]
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowsPackage {
    package_root: String,
    executable: String,
    version: String,
    package_full_name: String,
    package_family_name: String,
    application_id: String,
}

#[cfg(target_os = "windows")]
fn discover_windows() -> AppResult<CodexInstall> {
    let script = r#"
$package = Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction Stop |
  Where-Object { "$($_.SignatureKind)" -ieq 'Store' -and -not $_.IsDevelopmentMode } |
  Sort-Object Version -Descending | Select-Object -First 1
if (-not $package) { exit 3 }
$root = "$($package.InstallLocation)"
$exe = Join-Path $root 'app\ChatGPT.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) { exit 4 }
$manifest = Get-AppxPackageManifest -Package $package.PackageFullName
$application = @($manifest.Package.Applications.Application) | Where-Object {
  ("$($_.Executable)").Replace('/', '\') -ieq 'app\ChatGPT.exe'
} | Select-Object -First 1
if (-not $application -or -not $application.Id) { exit 5 }
[pscustomobject]@{
  packageRoot = $root
  executable = $exe
  version = "$($package.Version)"
  packageFullName = "$($package.PackageFullName)"
  packageFamilyName = "$($package.PackageFamilyName)"
  applicationId = "$($application.Id)"
} | ConvertTo-Json -Compress
"#;
    let mut command = Command::new("powershell.exe");
    suppress_windows_console(&mut command);
    let output = command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ])
        .output()?;
    if !output.status.success() {
        return Err(AppError::Runtime(
            "找不到可验证的官方 OpenAI.Codex Store 包".into(),
        ));
    }
    let package: WindowsPackage = serde_json::from_slice(&output.stdout)
        .map_err(|error| AppError::Runtime(format!("Codex 包信息无效：{error}")))?;
    let root = PathBuf::from(&package.package_root);
    let executable = PathBuf::from(&package.executable);
    let normalized_root = package.package_root.trim_end_matches(['\\', '/']);
    let normalized_executable = package.executable.replace('/', "\\");
    if !normalized_executable
        .to_ascii_lowercase()
        .starts_with(&format!(
            "{}\\",
            normalized_root.replace('/', "\\").to_ascii_lowercase()
        ))
        || !executable.is_file()
    {
        return Err(AppError::Runtime("Codex Store 包路径校验失败".into()));
    }
    let app_user_model_id =
        build_app_user_model_id(&package.package_family_name, &package.application_id)?;
    Ok(CodexInstall {
        platform: "windows".into(),
        executable,
        bundle_path: Some(root),
        version: package.version,
        identity: format!(
            "{}/{}",
            package.package_full_name, package.package_family_name
        ),
        app_user_model_id: Some(app_user_model_id),
    })
}

#[cfg(any(target_os = "windows", test))]
fn build_app_user_model_id(package_family_name: &str, application_id: &str) -> AppResult<String> {
    let safe_component = |value: &str, max_len: usize| {
        !value.is_empty()
            && value.len() <= max_len
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    };
    if !safe_component(package_family_name, 128) || !safe_component(application_id, 64) {
        return Err(AppError::Runtime(
            "Codex Store 包的应用用户模型 ID 无效".into(),
        ));
    }
    Ok(format!("{package_family_name}!{application_id}"))
}

#[cfg(any(target_os = "windows", test))]
fn cdp_activation_arguments(port: u16) -> String {
    format!("--remote-debugging-address=127.0.0.1 --remote-debugging-port={port}")
}

#[cfg(target_os = "windows")]
fn activate_windows_store_app(app_user_model_id: &str, arguments: &str) -> AppResult<u32> {
    let app_user_model_id = app_user_model_id.to_owned();
    let arguments = arguments.to_owned();
    std::thread::spawn(move || {
        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        initialized.ok().map_err(|error| {
            AppError::Runtime(format!("无法初始化 Windows 应用激活环境：{error}"))
        })?;
        struct ComGuard;
        impl Drop for ComGuard {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }
        let _guard = ComGuard;
        let manager: IApplicationActivationManager =
            unsafe { CoCreateInstance(&ApplicationActivationManager, None, CLSCTX_LOCAL_SERVER) }
                .map_err(|error| {
                AppError::Runtime(format!("无法创建 Windows Store 应用激活管理器：{error}"))
            })?;
        unsafe {
            manager.ActivateApplication(
                &HSTRING::from(app_user_model_id),
                &HSTRING::from(arguments),
                AO_NONE,
            )
        }
        .map_err(|error| AppError::Runtime(format!("Windows Store 应用激活失败：{error}")))
    })
    .join()
    .map_err(|_| AppError::Runtime("Windows Store 应用激活线程异常退出".into()))?
}

#[cfg(target_os = "windows")]
fn run_windows_identity_script(executable: &Path, script: &str) -> AppResult<bool> {
    let mut command = Command::new("powershell.exe");
    suppress_windows_console(&mut command);
    let output = command
        .env("LUMADROBE_CODEX_EXE", executable)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ])
        .output()?;
    if !output.status.success() {
        return Err(AppError::Runtime(format!(
            "Windows 进程身份检查失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim() == "true")
}

fn suppress_windows_console(command: &mut Command) {
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_port_defaults_do_not_overlap() {
        let install = CodexInstall {
            platform: "windows".into(),
            executable: PathBuf::from("ChatGPT.exe"),
            bundle_path: None,
            version: "1".into(),
            identity: "test".into(),
            app_user_model_id: Some("OpenAI.Codex_test!App".into()),
        };
        assert_eq!(install.preferred_port(), 9335);
        let mac = CodexInstall {
            platform: "macos".into(),
            ..install
        };
        assert_eq!(mac.preferred_port(), 9341);
    }

    #[test]
    fn windows_store_identity_and_cdp_arguments_are_strict() {
        assert_eq!(
            build_app_user_model_id("OpenAI.Codex_2p2nqsd0c76g0", "App").unwrap(),
            "OpenAI.Codex_2p2nqsd0c76g0!App"
        );
        assert!(build_app_user_model_id("OpenAI.Codex!", "App").is_err());
        assert!(build_app_user_model_id("OpenAI.Codex", "App With Space").is_err());
        assert_eq!(
            cdp_activation_arguments(9335),
            "--remote-debugging-address=127.0.0.1 --remote-debugging-port=9335"
        );
    }

    #[test]
    fn macos_listener_addresses_must_be_loopback_only() {
        assert!(macos_listener_is_loopback("127.0.0.1:9341", 9341));
        assert!(macos_listener_is_loopback("[::1]:9341", 9341));
        assert!(!macos_listener_is_loopback("*:9341", 9341));
        assert!(!macos_listener_is_loopback("0.0.0.0:9341", 9341));
        assert!(!macos_listener_is_loopback("127.0.0.1:9342", 9341));
    }

    #[test]
    fn macos_text_vnodes_require_the_exact_executable_path() {
        let executable = "/Applications/Codex.app/Contents/MacOS/ChatGPT";
        let output = format!(
            "p100\nftxt\nn{executable}\np101\nftxt\nn{executable}-spoof\np102\nftxt\nn{executable} (deleted)\n"
        );
        assert_eq!(parse_macos_text_pids(&output, executable), vec![100, 102]);
    }
}
