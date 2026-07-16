import { useCallback, useEffect, useMemo, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  Check,
  Download,
  GalleryVerticalEnd,
  Github,
  Import,
  LoaderCircle,
  Monitor,
  Palette,
  PlayCircle,
  RefreshCcw,
  RotateCcw,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react";
import {
  addSource,
  applyAndLaunch,
  deleteInstalledSkin,
  installSkin,
  getRuntimeStatus,
  listInstalledSkins,
  listSources,
  refreshSource,
  removeSource,
  restoreNative,
} from "./lib/api";
import type {
  CatalogSkin,
  InstalledSkin,
  RuntimeStatus,
  SourceRecord,
} from "./types";

type View = "wardrobe" | "import" | "restore";
type InstallationStatus = "remote" | "installed" | "update";

const featuredSkins: CatalogSkin[] = [
  {
    sourceId: "builtin-showcase",
    sourceName: "LumaDrobe Showcase",
    manifestPath: "skins/rose/skin.json",
    previewUrl:
      "https://raw.githubusercontent.com/yuebanddd/Codex-Dream-Skin/release/docs/images/gallery/skin-01.jpg",
    backgroundUrl:
      "https://raw.githubusercontent.com/yuebanddd/Codex-Dream-Skin/release/docs/images/gallery/skin-01.jpg",
    manifest: {
      schemaVersion: 1,
      id: "rose",
      name: "Rose Atelier / 玫瑰工坊",
      version: "1.0.0",
      author: "Dream Skin",
      description: "暖白与玫瑰色，给工作台一点轻盈的呼吸感。",
      background: "docs/images/gallery/skin-01.jpg",
      colors: { accent: "#a84758", secondary: "#2b2022" },
    },
  },
  {
    sourceId: "builtin-showcase",
    sourceName: "LumaDrobe Showcase",
    manifestPath: "skins/fiona/skin.json",
    previewUrl:
      "https://raw.githubusercontent.com/yuebanddd/Codex-Dream-Skin/release/docs/images/gallery/skin-07.jpg",
    backgroundUrl:
      "https://raw.githubusercontent.com/yuebanddd/Codex-Dream-Skin/release/docs/images/gallery/skin-07.jpg",
    manifest: {
      schemaVersion: 1,
      id: "fiona",
      name: "Dream / Fiona",
      version: "1.1.0",
      author: "Dream Skin",
      description: "紫粉色梦境，把灵感写进每一天。",
      background: "docs/images/gallery/skin-07.jpg",
      colors: { accent: "#8d3df0", secondary: "#ef64c8" },
    },
  },
  {
    sourceId: "builtin-showcase",
    sourceName: "LumaDrobe Showcase",
    manifestPath: "skins/stage/skin.json",
    previewUrl:
      "https://raw.githubusercontent.com/yuebanddd/Codex-Dream-Skin/release/docs/images/gallery/skin-08.jpg",
    backgroundUrl:
      "https://raw.githubusercontent.com/yuebanddd/Codex-Dream-Skin/release/docs/images/gallery/skin-08.jpg",
    manifest: {
      schemaVersion: 1,
      id: "stage",
      name: "Stage Black Gold / 舞台黑金",
      version: "1.0.3",
      author: "Dream Skin",
      description: "深色舞台与金色光点，适合长时间沉浸创作。",
      background: "docs/images/gallery/skin-08.jpg",
      colors: { accent: "#6542e8", secondary: "#09090e" },
    },
  },
];

function App() {
  const [view, setView] = useState<View>("wardrobe");
  const [sources, setSources] = useState<SourceRecord[]>([]);
  const [installed, setInstalled] = useState<InstalledSkin[]>([]);
  const [selectedId, setSelectedId] = useState("builtin-showcase:stage");
  const [loading, setLoading] = useState(true);
  const [working, setWorking] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [runtime, setRuntime] = useState<RuntimeStatus>({
    phase: "stopped",
    message: "正在连接 Rust Core",
  });

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [nextSources, nextInstalled, nextRuntime] = await Promise.all([
        listSources(),
        listInstalledSkins(),
        getRuntimeStatus(),
      ]);
      setSources(nextSources);
      setInstalled(nextInstalled);
      setRuntime(nextRuntime);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      void getRuntimeStatus().then(setRuntime).catch(() => undefined);
    }, 2000);
    return () => window.clearInterval(timer);
  }, []);

  const remoteSkins = useMemo(
    () => sources.flatMap((source) => source.skins),
    [sources],
  );
  const catalogSkins = useMemo(() => {
    const installedByKey = new Map(
      installed.map((skin) => [`${skin.sourceId}:${skin.skinId}`, skin]),
    );
    const remoteKeys = new Set(
      remoteSkins.map((skin) => `${skin.sourceId}:${skin.manifest.id}`),
    );
    const subscribedSkins = remoteSkins.map((skin) => {
      const local = installedByKey.get(`${skin.sourceId}:${skin.manifest.id}`);
      if (!local || local.version !== skin.manifest.version) return skin;
      return {
        ...skin,
        previewUrl: convertFileSrc(local.previewPath ?? local.backgroundPath),
        backgroundUrl: convertFileSrc(local.backgroundPath),
        cssUrl: local.cssPath ? convertFileSrc(local.cssPath) : undefined,
      };
    });
    const localOnlySkins = installed
      .filter((skin) => !remoteKeys.has(`${skin.sourceId}:${skin.skinId}`))
      .map<CatalogSkin>((skin) => ({
        sourceId: skin.sourceId,
        sourceName: skin.sourceName,
        manifestPath: "local://skin.json",
        previewUrl: convertFileSrc(skin.previewPath ?? skin.backgroundPath),
        backgroundUrl: convertFileSrc(skin.backgroundPath),
        cssUrl: skin.cssPath ? convertFileSrc(skin.cssPath) : undefined,
        manifest: skin.manifest,
      }));
    return [...subscribedSkins, ...localOnlySkins];
  }, [installed, remoteSkins]);
  const skins = catalogSkins.length ? catalogSkins : featuredSkins;
  const skinKey = (skin: CatalogSkin) => `${skin.sourceId}:${skin.manifest.id}`;
  const selected =
    skins.find((skin) => skinKey(skin) === selectedId) ?? skins[0];
  const installedVersions = useMemo(
    () =>
      new Map(
        installed.map((skin) => [
          `${skin.sourceId}:${skin.skinId}`,
          skin.version,
        ]),
      ),
    [installed],
  );

  async function handleAdd(repositoryUrl: string) {
    setWorking("add");
    setError("");
    try {
      const source = await addSource(repositoryUrl);
      setSources((current) => [
        ...current.filter((item) => item.id !== source.id),
        source,
      ]);
      if (source.skins[0]) setSelectedId(skinKey(source.skins[0]));
      setView("wardrobe");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(null);
    }
  }

  async function handleRefresh(sourceId: string) {
    setWorking(sourceId);
    try {
      const source = await refreshSource(sourceId);
      setSources((current) =>
        current.map((item) => (item.id === source.id ? source : item)),
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(null);
    }
  }

  async function handleRemove(sourceId: string) {
    setWorking(sourceId);
    try {
      await removeSource(sourceId);
      setSources((current) => current.filter((item) => item.id !== sourceId));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(null);
    }
  }

  async function handleInstall(skin: CatalogSkin) {
    const key = skinKey(skin);
    setWorking(`install:${key}`);
    setError("");
    try {
      const result = await installSkin(skin.sourceId, skin.manifest.id);
      setInstalled((current) => [
        ...current.filter(
          (item) =>
            item.sourceId !== result.sourceId || item.skinId !== result.skinId,
        ),
        result,
      ]);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(null);
    }
  }

  async function handleDelete(skin: CatalogSkin) {
    const key = skinKey(skin);
    setWorking(`delete:${key}`);
    setError("");
    try {
      await deleteInstalledSkin(skin.sourceId, skin.manifest.id);
      setInstalled((current) =>
        current.filter(
          (item) =>
            item.sourceId !== skin.sourceId || item.skinId !== skin.manifest.id,
        ),
      );
    } catch (reason) {
      setError(String(reason));
    } finally {
      setWorking(null);
    }
  }

  async function handleApply(skin: CatalogSkin) {
    const key = skinKey(skin);
    setWorking(`apply:${key}`);
    setError("");
    setRuntime((current) => ({
      ...current,
      phase: "starting",
      activeSourceId: skin.sourceId,
      activeSkinId: skin.manifest.id,
      activeVersion: skin.manifest.version,
      message: "正在启动 Codex 与回环 CDP",
    }));
    try {
      setRuntime(await applyAndLaunch(skin.sourceId, skin.manifest.id));
    } catch (reason) {
      setError(String(reason));
      setRuntime(await getRuntimeStatus());
    } finally {
      setWorking(null);
    }
  }

  async function handleRestore() {
    setWorking("restore");
    setError("");
    try {
      setRuntime(await restoreNative());
    } catch (reason) {
      setError(String(reason));
      setRuntime(await getRuntimeStatus());
    } finally {
      setWorking(null);
    }
  }

  return (
    <div className="atelier-shell">
      <Sidebar
        view={view}
        count={installed.length}
        runtime={runtime}
        onNavigate={setView}
      />
      <main className="atelier-main">
        {error && <div className="error-toast">{error}</div>}
        {loading ? (
          <div className="loading-state">
            <LoaderCircle className="spin" />
            <span>正在整理主题衣橱…</span>
          </div>
        ) : view === "wardrobe" ? (
          <Wardrobe
            skins={skins}
            selected={selected}
            installedVersions={installedVersions}
            working={working}
            runtime={runtime}
            onSelect={setSelectedId}
            onImport={() => setView("import")}
            onInstall={handleInstall}
            onApply={handleApply}
            onDelete={handleDelete}
          />
        ) : view === "import" ? (
          <ImportSources
            sources={sources}
            working={working}
            onAdd={handleAdd}
            onRefresh={handleRefresh}
            onRemove={handleRemove}
          />
        ) : (
          <RestoreView
            runtime={runtime}
            working={working === "restore"}
            onRestore={handleRestore}
          />
        )}
      </main>
    </div>
  );
}

function Sidebar({
  view,
  count,
  runtime,
  onNavigate,
}: {
  view: View;
  count: number;
  runtime: RuntimeStatus;
  onNavigate: (view: View) => void;
}) {
  return (
    <aside className="atelier-sidebar">
      <div className="window-dots">
        <i />
        <i />
        <i />
      </div>
      <div className="atelier-brand">
        <div className="logo-mark">
          <Palette size={22} />
        </div>
        <div>
          <strong>LumaDrobe</strong>
          <span>THEME ATELIER</span>
        </div>
      </div>
      <div className="language-switch">
        <button className="active">中文</button>
        <button>EN</button>
      </div>
      <nav className="atelier-nav">
        <button
          className={view === "wardrobe" ? "active" : ""}
          onClick={() => onNavigate("wardrobe")}
        >
          <GalleryVerticalEnd />
          <span>主题衣橱</span>
          <b>{count}</b>
        </button>
        <button
          className={view === "import" ? "active" : ""}
          onClick={() => onNavigate("import")}
        >
          <Import />
          <span>导入主题</span>
        </button>
        <button
          className={view === "restore" ? "active" : ""}
          onClick={() => onNavigate("restore")}
        >
          <RotateCcw />
          <span>恢复原生</span>
        </button>
      </nav>
      <p className="safety-copy">
        主题只改变外观，不修改 Codex 安装包和你的对话数据。
      </p>
      <div className={`runtime-card ${runtime.phase}`}>
        <i />
        <div>
          <strong>
            {runtime.phase === "running"
              ? "皮肤引擎正在运行"
              : runtime.phase === "error"
                ? "皮肤引擎需要处理"
                : runtime.phase === "starting" || runtime.phase === "checking"
                  ? "正在验证皮肤引擎"
                  : "皮肤引擎已就绪"}
          </strong>
          <span>
            Rust Core · {runtime.port ? `CDP ${runtime.port}` : "CDP loopback"}
          </span>
        </div>
        <Monitor size={17} />
      </div>
    </aside>
  );
}

function Wardrobe({
  skins,
  selected,
  installedVersions,
  working,
  runtime,
  onSelect,
  onImport,
  onInstall,
  onApply,
  onDelete,
}: {
  skins: CatalogSkin[];
  selected: CatalogSkin;
  installedVersions: Map<string, string>;
  working: string | null;
  runtime: RuntimeStatus;
  onSelect: (id: string) => void;
  onImport: () => void;
  onInstall: (skin: CatalogSkin) => Promise<void>;
  onApply: (skin: CatalogSkin) => Promise<void>;
  onDelete: (skin: CatalogSkin) => Promise<void>;
}) {
  const selectedKey = `${selected.sourceId}:${selected.manifest.id}`;
  const activeKey = runtime.activeSourceId && runtime.activeSkinId
    ? `${runtime.activeSourceId}:${runtime.activeSkinId}`
    : undefined;
  const getStatus = (skin: CatalogSkin): InstallationStatus => {
    const installedVersion = installedVersions.get(
      `${skin.sourceId}:${skin.manifest.id}`,
    );
    if (!installedVersion) return "remote";
    return installedVersion === skin.manifest.version ? "installed" : "update";
  };
  return (
    <div className="wardrobe-layout">
      <section className="wardrobe-content">
        <header className="wardrobe-header">
          <div>
            <span className="atelier-eyebrow">
              <Sparkles size={13} /> 为你的 Codex 精选
            </span>
            <h1>给工作台，换一种心情。</h1>
            <p>挑一套主题，一键启动。你的项目、任务和习惯都保持原样。</p>
          </div>
          <button className="import-button" onClick={onImport}>
            <Download size={18} />
            导入主题包
          </button>
        </header>
        <div className="series-heading">
          <span>01</span>
          <h2>主题系列</h2>
          <small>{skins.length} 件可用主题</small>
        </div>
        <div className="theme-grid">
          {skins.map((skin) => (
            <ThemeCard
              key={`${skin.sourceId}:${skin.manifest.id}`}
              skin={skin}
              selected={`${skin.sourceId}:${skin.manifest.id}` === selectedKey}
              status={getStatus(skin)}
              active={activeKey === `${skin.sourceId}:${skin.manifest.id}` && runtime.phase === "running"}
              onSelect={() => onSelect(`${skin.sourceId}:${skin.manifest.id}`)}
            />
          ))}
        </div>
      </section>
      <FittingRoom
        skin={selected}
        status={getStatus(selected)}
        working={working}
        active={activeKey === selectedKey && runtime.phase === "running"}
        onInstall={onInstall}
        onApply={onApply}
        onDelete={onDelete}
      />
    </div>
  );
}

function ThemeCard({
  skin,
  selected,
  status,
  active,
  onSelect,
}: {
  skin: CatalogSkin;
  selected: boolean;
  status: InstallationStatus;
  active: boolean;
  onSelect: () => void;
}) {
  const colors = Object.values(skin.manifest.colors ?? {}).slice(0, 3);
  return (
    <button
      className={selected ? "theme-card selected" : "theme-card"}
      onClick={onSelect}
    >
      <div
        className="theme-art"
        style={{
          backgroundImage: `url(${skin.previewUrl ?? skin.backgroundUrl})`,
        }}
      >
        <span
          className={status === "remote" ? "remote-badge" : "imported-badge"}
        >
          {status === "installed"
            ? "已安装"
            : status === "update"
              ? "可更新"
              : "云端"}
        </span>
        {(selected || active) && (
          <span className={`using-badge ${active ? "active" : ""}`}>
            {active ? "使用中" : "已选择"}
          </span>
        )}
        <div className="theme-title">
          <strong>{skin.manifest.name}</strong>
          <span>v{skin.manifest.version}</span>
        </div>
      </div>
      <footer>
        <p>{skin.manifest.description ?? "为你的 Codex 工作台准备的主题。"}</p>
        <div className="swatches">
          {(colors.length ? colors : ["#6e45e8", "#0b0a11"]).map((color) => (
            <i key={color} style={{ background: color }} />
          ))}
        </div>
      </footer>
    </button>
  );
}

function FittingRoom({
  skin,
  status,
  working,
  active,
  onInstall,
  onApply,
  onDelete,
}: {
  skin: CatalogSkin;
  status: InstallationStatus;
  working: string | null;
  active: boolean;
  onInstall: (skin: CatalogSkin) => Promise<void>;
  onApply: (skin: CatalogSkin) => Promise<void>;
  onDelete: (skin: CatalogSkin) => Promise<void>;
}) {
  const colors = Object.values(skin.manifest.colors ?? {}).slice(0, 3);
  const key = `${skin.sourceId}:${skin.manifest.id}`;
  const installing = working === `install:${key}`;
  const deleting = working === `delete:${key}`;
  const applying = working === `apply:${key}`;
  const themeOperationActive =
    working?.startsWith("install:") ||
    working?.startsWith("delete:") ||
    working?.startsWith("apply:") ||
    working === "restore";
  const showcase = skin.sourceId === "builtin-showcase";
  const installed = status === "installed";
  const hasLocalVersion = status !== "remote";
  const updateAvailable = status === "update";
  return (
    <aside className="fitting-room">
      <header>
        <h2>试衣镜</h2>
        <span>
          {updateAvailable
            ? "发现新版本"
            : installed
              ? active
                ? "当前使用中"
                : "本地已安装"
              : "在线主题"}
        </span>
      </header>
      <div
        className="poster"
        style={{
          backgroundImage: `url(${skin.previewUrl ?? skin.backgroundUrl})`,
        }}
      >
        <div>
          <small>{hasLocalVersion ? "LOCAL EDITION" : "SOURCE PREVIEW"}</small>
          <strong>{skin.manifest.name}</strong>
          <p>{skin.manifest.description}</p>
        </div>
      </div>
      <div className="color-row">
        <span>主题色</span>
        <div>
          {(colors.length ? colors : ["#6e45e8", "#eee9df", "#0a0910"]).map(
            (color) => (
              <i key={color} style={{ background: color }} />
            ),
          )}
        </div>
        <small>v{skin.manifest.version}</small>
      </div>
      <div className="fitting-actions">
        <button
          className="apply-button"
          disabled={showcase || themeOperationActive}
          onClick={() =>
            void (installed ? onApply(skin) : onInstall(skin))
          }
        >
          {installing || applying ? (
            <LoaderCircle className="spin" size={20} />
          ) : installed ? (
            active ? <Check size={20} /> : <PlayCircle size={20} />
          ) : (
            <Download size={20} />
          )}
          <span>
            <strong>
              {installing
                ? "正在安全导入"
                : applying
                  ? "正在应用并启动"
                : installed
                  ? active
                    ? "重新应用主题"
                    : "应用并启动"
                  : updateAvailable
                    ? `更新到 v${skin.manifest.version}`
                    : showcase
                      ? "订阅后安装"
                      : "下载并安装"}
            </strong>
            <small>
              {installed
                ? active
                  ? "热重载当前主题"
                  : "Rust Core · loopback CDP"
                : updateAvailable
                  ? "安全替换本地版本"
                  : "校验图片与 CSS"}
            </small>
          </span>
        </button>
        <button disabled>
          <Upload size={18} />
          <span>导出</span>
        </button>
        <button
          className="delete-button"
          disabled={!hasLocalVersion || active || themeOperationActive}
          onClick={() => void onDelete(skin)}
        >
          {deleting ? (
            <LoaderCircle className="spin" size={18} />
          ) : (
            <Trash2 size={18} />
          )}
          <span>删除</span>
        </button>
      </div>
    </aside>
  );
}

function ImportSources({
  sources,
  working,
  onAdd,
  onRefresh,
  onRemove,
}: {
  sources: SourceRecord[];
  working: string | null;
  onAdd: (url: string) => Promise<void>;
  onRefresh: (id: string) => Promise<void>;
  onRemove: (id: string) => Promise<void>;
}) {
  const [url, setUrl] = useState(
    "https://github.com/yuebanddd/Codex-Dream-Skin?ref=release",
  );
  return (
    <div className="source-page">
      <span className="atelier-eyebrow">
        <Github size={13} /> Git Repository Sources
      </span>
      <h1>导入主题源</h1>
      <p>粘贴任意符合 Dream Skin 协议的公开 GitHub 仓库地址。</p>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void onAdd(url);
        }}
      >
        <Github size={20} />
        <input value={url} onChange={(event) => setUrl(event.target.value)} />
        <button disabled={working === "add"}>
          {working === "add" ? <LoaderCircle className="spin" /> : <Download />}
          订阅仓库
        </button>
      </form>
      <div className="repository-list">
        {sources.map((source) => (
          <article key={source.id}>
            <div className="repo-icon">
              <Github />
            </div>
            <div>
              <strong>{source.name}</strong>
              <span>
                {source.owner}/{source.repository} · {source.refName}
              </span>
              <small>{source.skins.length} 件主题</small>
            </div>
            <button onClick={() => void onRefresh(source.id)}>
              <RefreshCcw className={working === source.id ? "spin" : ""} />
            </button>
            <button className="danger" onClick={() => void onRemove(source.id)}>
              <Trash2 />
            </button>
          </article>
        ))}
      </div>
    </div>
  );
}

function RestoreView({
  runtime,
  working,
  onRestore,
}: {
  runtime: RuntimeStatus;
  working: boolean;
  onRestore: () => Promise<void>;
}) {
  const canRestore = runtime.phase !== "stopped";
  return (
    <div className="restore-page">
      <div className="restore-icon">
        <RotateCcw />
      </div>
      <span className="atelier-eyebrow">SAFE RESTORE</span>
      <h1>恢复 Codex 原生外观</h1>
      <p>
        停止皮肤引擎、移除当前注入，并按原子备份恢复外观设置。项目与对话不会受到影响。
      </p>
      <button
        disabled={!canRestore || working}
        onClick={() => void onRestore()}
      >
        {working ? <LoaderCircle className="spin" /> : <RotateCcw />}
        {working ? "正在安全恢复" : "恢复原生并重启"}
      </button>
      <div className="restore-note">
        {runtime.phase === "error" ? <Monitor /> : <Check />}
        {runtime.message}
      </div>
    </div>
  );
}

export default App;
