export type ViewId = "discover" | "sources" | "library" | "create" | "status";

export interface SkinManifest {
  schemaVersion: number;
  id: string;
  name: string;
  version: string;
  description?: string;
  author: string;
  engineVersion?: string;
  platforms?: string[];
  preview?: string;
  background: string;
  css?: string;
  colors?: Record<string, string>;
}

export interface CatalogSkin {
  sourceId: string;
  sourceName: string;
  manifestPath: string;
  previewUrl?: string;
  backgroundUrl: string;
  cssUrl?: string;
  manifest: SkinManifest;
}

export interface SourceRecord {
  id: string;
  repositoryUrl: string;
  owner: string;
  repository: string;
  refName: string;
  name: string;
  author: string;
  description?: string;
  refreshedAt: string;
  skins: CatalogSkin[];
}

export interface InstalledSkin {
  sourceId: string;
  sourceName: string;
  skinId: string;
  name: string;
  version: string;
  author: string;
  installedAt: string;
  installDir: string;
  backgroundPath: string;
  previewPath?: string;
  cssPath?: string;
  assetHashes: Record<string, string>;
  manifest: SkinManifest;
}

export interface RuntimeStatus {
  phase:
    | "stopped"
    | "checking"
    | "starting"
    | "running"
    | "pausing"
    | "paused"
    | "stopping"
    | "error";
  activeSourceId?: string;
  activeSkinId?: string;
  activeVersion?: string;
  port?: number;
  message: string;
}

export interface RuntimeDiagnostics {
  generatedAt: string;
  clientVersion: string;
  platform: string;
  architecture: string;
  runtime: RuntimeStatus;
  savedSession: boolean;
  paused: boolean;
  codexFound: boolean;
  codexVersion?: string;
  codexIdentity?: string;
  executable?: string;
  codexRunning?: boolean;
  listenerVerified?: boolean;
  endpointVerified?: boolean;
  verifiedTargets?: number;
  notes: string[];
}
