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
  RefreshCcw,
  RotateCcw,
  Sparkles,
  Trash2,
  Upload,
} from "lucide-react";
import {
  addSource,
  deleteInstalledSkin,
  installSkin,
  listInstalledSkins,
  listSources,
  refreshSource,
  removeSource,
} from "./lib/api";
import type { CatalogSkin, InstalledSkin, SourceRecord } from "./types";

type View = "wardrobe" | "import" | "restore";

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

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [nextSources, nextInstalled] = await Promise.all([
        listSources(),
        listInstalledSkins(),
      ]);
      setSources(nextSources);
      setInstalled(nextInstalled);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const remoteSkins = useMemo(
    () => sources.flatMap((source) => source.skins),
    [sources],
  );
  const installedOnlySkins = useMemo(() => {
    const remoteKeys = new Set(
      remoteSkins.map((skin) => `${skin.sourceId}:${skin.manifest.id}`),
    );
    return installed
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
  }, [installed, remoteSkins]);
  const catalogSkins = [...remoteSkins, ...installedOnlySkins];
  const skins = catalogSkins.length ? catalogSkins : featuredSkins;
  const skinKey = (skin: CatalogSkin) => `${skin.sourceId}:${skin.manifest.id}`;
  const selected =
    skins.find((skin) => skinKey(skin) === selectedId) ?? skins[0];
  const installedKeys = useMemo(
    () => new Set(installed.map((skin) => `${skin.sourceId}:${skin.skinId}`)),
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

  return (
    <div className="atelier-shell">
      <Sidebar view={view} count={installed.length} onNavigate={setView} />
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
            installedKeys={installedKeys}
            working={working}
            onSelect={setSelectedId}
            onImport={() => setView("import")}
            onInstall={handleInstall}
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
          <RestoreView />
        )}
      </main>
    </div>
  );
}

function Sidebar({
  view,
  count,
  onNavigate,
}: {
  view: View;
  count: number;
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
      <div className="runtime-card">
        <i />
        <div>
          <strong>皮肤引擎待接入</strong>
          <span>Rust Core · CDP</span>
        </div>
        <Monitor size={17} />
      </div>
    </aside>
  );
}

function Wardrobe({
  skins,
  selected,
  installedKeys,
  working,
  onSelect,
  onImport,
  onInstall,
  onDelete,
}: {
  skins: CatalogSkin[];
  selected: CatalogSkin;
  installedKeys: Set<string>;
  working: string | null;
  onSelect: (id: string) => void;
  onImport: () => void;
  onInstall: (skin: CatalogSkin) => Promise<void>;
  onDelete: (skin: CatalogSkin) => Promise<void>;
}) {
  const selectedKey = `${selected.sourceId}:${selected.manifest.id}`;
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
              installed={installedKeys.has(
                `${skin.sourceId}:${skin.manifest.id}`,
              )}
              onSelect={() => onSelect(`${skin.sourceId}:${skin.manifest.id}`)}
            />
          ))}
        </div>
      </section>
      <FittingRoom
        skin={selected}
        installed={installedKeys.has(selectedKey)}
        working={working}
        onInstall={onInstall}
        onDelete={onDelete}
      />
    </div>
  );
}

function ThemeCard({
  skin,
  selected,
  installed,
  onSelect,
}: {
  skin: CatalogSkin;
  selected: boolean;
  installed: boolean;
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
        <span className={installed ? "imported-badge" : "remote-badge"}>
          {installed ? "已安装" : "云端"}
        </span>
        {selected && <span className="using-badge">已选择</span>}
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
  installed,
  working,
  onInstall,
  onDelete,
}: {
  skin: CatalogSkin;
  installed: boolean;
  working: string | null;
  onInstall: (skin: CatalogSkin) => Promise<void>;
  onDelete: (skin: CatalogSkin) => Promise<void>;
}) {
  const colors = Object.values(skin.manifest.colors ?? {}).slice(0, 3);
  const key = `${skin.sourceId}:${skin.manifest.id}`;
  const installing = working === `install:${key}`;
  const deleting = working === `delete:${key}`;
  const showcase = skin.sourceId === "builtin-showcase";
  return (
    <aside className="fitting-room">
      <header>
        <h2>试衣镜</h2>
        <span>{installed ? "本地已安装" : "在线主题"}</span>
      </header>
      <div
        className="poster"
        style={{
          backgroundImage: `url(${skin.previewUrl ?? skin.backgroundUrl})`,
        }}
      >
        <div>
          <small>{installed ? "LOCAL EDITION" : "SOURCE PREVIEW"}</small>
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
          disabled={installed || showcase || installing}
          onClick={() => void onInstall(skin)}
        >
          {installing ? (
            <LoaderCircle className="spin" size={20} />
          ) : installed ? (
            <Check size={20} />
          ) : (
            <Download size={20} />
          )}
          <span>
            <strong>
              {installing
                ? "正在安全导入"
                : installed
                  ? "已安装到本地"
                  : showcase
                    ? "订阅后安装"
                    : "下载并安装"}
            </strong>
            <small>{installed ? "第三轮接入启动" : "校验图片与 CSS"}</small>
          </span>
        </button>
        <button disabled>
          <Upload size={18} />
          <span>导出</span>
        </button>
        <button
          className="delete-button"
          disabled={!installed || deleting}
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

function RestoreView() {
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
      <button>
        <RotateCcw />
        恢复原生并重启
      </button>
      <div className="restore-note">
        <Check />
        现有 macOS / Windows 恢复脚本将在下一阶段迁移到 Rust Core。
      </div>
    </div>
  );
}

export default App;
