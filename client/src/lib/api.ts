import { invoke } from "@tauri-apps/api/core";
import type {
  CatalogSkin,
  InstalledSkin,
  RuntimeDiagnostics,
  RuntimeStatus,
  SourceRecord,
} from "../types";

const demoSource: SourceRecord = {
  id: "dream-skin-demo",
  repositoryUrl: "https://github.com/yuebanddd/Codex-Dream-Skin",
  owner: "yuebanddd",
  repository: "Codex-Dream-Skin",
  refName: "release",
  name: "LumaDrobe 官方示例源",
  author: "LumaDrobe contributors",
  description: "启动桌面端后即可刷新 GitHub 仓库中的真实皮肤目录。",
  refreshedAt: new Date().toISOString(),
  skins: [],
};

function inTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

export async function listSources(): Promise<SourceRecord[]> {
  if (!inTauri()) return [demoSource];
  return invoke<SourceRecord[]>("list_sources");
}

export async function addSource(repositoryUrl: string): Promise<SourceRecord> {
  if (!inTauri()) {
    throw new Error(
      "Web 预览模式无法访问本机 Rust 服务，请使用 Tauri 桌面端。 ",
    );
  }
  return invoke<SourceRecord>("add_source", { repositoryUrl });
}

export async function refreshSource(sourceId: string): Promise<SourceRecord> {
  return invoke<SourceRecord>("refresh_source", { sourceId });
}

export async function removeSource(sourceId: string): Promise<void> {
  return invoke("remove_source", { sourceId });
}

export async function listCatalogSkins(): Promise<CatalogSkin[]> {
  if (!inTauri()) return [];
  return invoke<CatalogSkin[]>("list_catalog_skins");
}

export async function listInstalledSkins(): Promise<InstalledSkin[]> {
  if (!inTauri()) return [];
  return invoke<InstalledSkin[]>("list_installed_skins");
}

export async function installSkin(
  sourceId: string,
  skinId: string,
): Promise<InstalledSkin> {
  if (!inTauri()) {
    throw new Error("Web 预览模式不能写入本地主题库，请使用 Tauri 桌面端。");
  }
  return invoke<InstalledSkin>("install_skin", { sourceId, skinId });
}

export async function deleteInstalledSkin(
  sourceId: string,
  skinId: string,
): Promise<void> {
  return invoke("delete_installed_skin", { sourceId, skinId });
}

export async function getRuntimeStatus(): Promise<RuntimeStatus> {
  if (!inTauri()) {
    return { phase: "stopped", message: "Web 预览模式 · Rust Core 未连接" };
  }
  return invoke<RuntimeStatus>("runtime_status");
}

export async function getRuntimeDiagnostics(): Promise<RuntimeDiagnostics> {
  if (!inTauri()) {
    return {
      generatedAt: new Date().toISOString(),
      clientVersion: "web-preview",
      platform: navigator.platform || "web",
      architecture: "unknown",
      runtime: await getRuntimeStatus(),
      savedSession: false,
      paused: false,
      codexFound: false,
      notes: ["Web 预览模式无法读取本机 Codex 诊断信息。"],
    };
  }
  return invoke<RuntimeDiagnostics>("runtime_diagnostics");
}

export async function applyAndLaunch(
  sourceId: string,
  skinId: string,
): Promise<RuntimeStatus> {
  if (!inTauri()) {
    throw new Error("Web 预览模式不能启动 Codex，请使用 Tauri 桌面端。");
  }
  return invoke<RuntimeStatus>("apply_and_launch", { sourceId, skinId });
}

export async function restoreNative(): Promise<RuntimeStatus> {
  if (!inTauri()) {
    throw new Error("Web 预览模式不能恢复 Codex，请使用 Tauri 桌面端。");
  }
  return invoke<RuntimeStatus>("restore_native");
}

export async function pauseTheme(): Promise<RuntimeStatus> {
  if (!inTauri()) {
    throw new Error("Web 预览模式不能暂停皮肤，请使用 Tauri 桌面端。");
  }
  return invoke<RuntimeStatus>("pause_theme");
}

export async function resumeTheme(): Promise<RuntimeStatus> {
  if (!inTauri()) {
    throw new Error("Web 预览模式不能恢复皮肤，请使用 Tauri 桌面端。");
  }
  return invoke<RuntimeStatus>("resume_theme");
}
