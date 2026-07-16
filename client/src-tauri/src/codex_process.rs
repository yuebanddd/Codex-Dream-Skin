use crate::error::{AppError, AppResult};
#[cfg(target_os = "windows")]
use serde::Deserialize;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

const EXPECTED_MAC_TEAM_ID: &str = "2DC432GLL2";

#[derive(Debug, Clone)]
pub struct CodexInstall {
    pub platform: String,
    pub executable: PathBuf,
    pub bundle_path: Option<PathBuf>,
    pub version: String,
    pub identity: String,
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

    pub fn launch_with_cdp(&self, port: u16) -> AppResult<Child> {
        Command::new(&self.executable)
            .arg("--remote-debugging-address=127.0.0.1")
            .arg(format!("--remote-debugging-port={port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(AppError::Io)
    }

    pub fn launch_normally(&self) -> AppResult<()> {
        #[cfg(target_os = "macos")]
        {
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
            Command::new(&self.executable)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?;
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
        if let Some(child) = launched_child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }

        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("/usr/bin/osascript")
                .args(["-e", "tell application id \"com.openai.codex\" to quit"])
                .status();
            let deadline = Instant::now() + Duration::from_secs(15);
            while self.is_running()? && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(250));
            }
            if self.is_running()? {
                return Err(AppError::Runtime(
                    "Codex 未能安全退出；未强制结束进程，请手动退出后重试".into(),
                ));
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
fn discover_macos() -> AppResult<CodexInstall> {
    let mut candidates = vec![PathBuf::from("/Applications/ChatGPT.app")];
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join("Applications/ChatGPT.app"));
    }
    for bundle in candidates {
        let plist = bundle.join("Contents/Info.plist");
        if !plist.is_file() {
            continue;
        }
        let identifier = plutil_value(&plist, "CFBundleIdentifier")?;
        if identifier != "com.openai.codex" {
            continue;
        }
        verify_macos_signature(&bundle)?;
        let executable_name = plutil_value(&plist, "CFBundleExecutable")?;
        let version = plutil_value(&plist, "CFBundleShortVersionString")?;
        let executable = bundle.join("Contents/MacOS").join(executable_name);
        if !executable.is_file() {
            return Err(AppError::Runtime("官方 Codex 可执行文件不存在".into()));
        }
        return Ok(CodexInstall {
            platform: "macos".into(),
            executable,
            bundle_path: Some(bundle),
            version,
            identity: format!("com.openai.codex/{EXPECTED_MAC_TEAM_ID}"),
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
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,command="])
        .output()?;
    let expected = executable.to_string_lossy();
    let pids = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            let split = trimmed.find(char::is_whitespace)?;
            let pid = trimmed[..split].parse::<u32>().ok()?;
            let command = trimmed[split..].trim_start();
            command.starts_with(expected.as_ref()).then_some(pid)
        })
        .collect();
    Ok(pids)
}

#[cfg(target_os = "macos")]
fn verify_macos_listener_owner(executable: &Path, port: u16) -> AppResult<bool> {
    let output = Command::new("/usr/sbin/lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()?;
    if !output.status.success() && output.stdout.is_empty() {
        return Ok(false);
    }
    let pids: Vec<u32> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .collect();
    if pids.is_empty() {
        return Ok(false);
    }
    for pid in pids {
        if !macos_pid_descends_from_executable(pid, executable)? {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(target_os = "macos")]
fn macos_pid_descends_from_executable(mut pid: u32, executable: &Path) -> AppResult<bool> {
    let expected = executable.to_string_lossy();
    for _ in 0..32 {
        if pid <= 1 {
            return Ok(false);
        }
        let output = Command::new("/bin/ps")
            .args(["-p", &pid.to_string(), "-o", "ppid=,command="])
            .output()?;
        let line = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let Some(split) = line.find(char::is_whitespace) else {
            return Ok(false);
        };
        let parent = line[..split].trim().parse::<u32>().unwrap_or_default();
        let command = line[split..].trim_start();
        if command.starts_with(expected.as_ref()) {
            return Ok(true);
        }
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
[pscustomobject]@{
  packageRoot = $root
  executable = $exe
  version = "$($package.Version)"
  packageFullName = "$($package.PackageFullName)"
  packageFamilyName = "$($package.PackageFamilyName)"
} | ConvertTo-Json -Compress
"#;
    let output = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
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
    Ok(CodexInstall {
        platform: "windows".into(),
        executable,
        bundle_path: Some(root),
        version: package.version,
        identity: format!(
            "{}/{}",
            package.package_full_name, package.package_family_name
        ),
    })
}

#[cfg(target_os = "windows")]
fn run_windows_identity_script(executable: &Path, script: &str) -> AppResult<bool> {
    let output = Command::new("powershell.exe")
        .env("LUMADROBE_CODEX_EXE", executable)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
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
        };
        assert_eq!(install.preferred_port(), 9335);
        let mac = CodexInstall {
            platform: "macos".into(),
            ..install
        };
        assert_eq!(mac.preferred_port(), 9341);
    }
}
