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
