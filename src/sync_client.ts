import { invoke } from "@tauri-apps/api/core";

export interface SyncStatus {
  accepted_operations: number;
  rejected_operations: number;
  received_changes: number;
  conflicts: number;
  has_more: boolean;
  blocked: boolean;
}

// Integer protocol values are decimal strings, including revisions.
export type SyncValue =
  | { type: "integer"; value: string }
  | { type: "real"; value: number }
  | { type: "text" | "date_time" | "uuid"; value: string }
  | { type: "binary"; value: number[] }
  | { type: "boolean"; value: boolean }
  | { type: "null" };
export type SyncRecord = Record<string, SyncValue>;

export interface SyncEntity {
  model: string;
  id: string;
  schema_version: number;
  revision: string;
  epoch: string | null;
  updated_at: string | null;
  deleted_at: string | null;
  record: SyncRecord | null;
}

export interface SyncConflict {
  model: string;
  id: string;
  base: SyncRecord | null;
  local: SyncRecord | null;
  remote: SyncEntity;
  fields: string[];
  reason: string | null;
  retryable: boolean;
  acknowledgement?: SyncEntity;
}

export type SyncResolution =
  | { strategy: "local" | "remote" }
  | { strategy: "record"; record: SyncRecord | null };
export interface ObjectAccessStatus {
  model: string;
  id: string;
  rights: { read: boolean; change: boolean; delete: boolean };
  status: "active" | "revoked" | "deleted" | "review" | "discarded";
  has_draft: boolean;
  submitted: boolean;
}
export type SyncRequest =
  | { action: "pull" | "flush" | "sync_once" | "conflicts" | "object_access" }
  | { action: "retained_draft" | "resume_object" | "discard_retained_draft"; model: string; id: string }
  | { action: "resolve"; model: string; id: string; resolution: SyncResolution };
export type SyncResponse =
  | { kind: "report"; data: SyncStatus }
  | { kind: "conflicts"; data: SyncConflict[] }
  | { kind: "object_access"; data: ObjectAccessStatus[] }
  | { kind: "retained_draft"; data: SyncRecord | null }
  | { kind: "resolved" };

async function report(action: "pull" | "flush" | "sync_once"): Promise<SyncStatus> {
  const response = await invoke<SyncResponse>("che_model_sync", { request: { action } });
  if (response.kind !== "report") throw new Error("Unexpected sync response");
  return response.data;
}

export const modelSync = {
  pull: () => report("pull"),
  flush: () => report("flush"),
  syncOnce: () => report("sync_once"),
  async objectAccess(): Promise<ObjectAccessStatus[]> {
    const response = await invoke<SyncResponse>("che_model_sync", { request: { action: "object_access" } });
    if (response.kind !== "object_access") throw new Error("Unexpected sync response");
    return response.data;
  },
  async retainedDraft(model: string, id: string): Promise<SyncRecord | null> {
    const response = await invoke<SyncResponse>("che_model_sync", { request: { action: "retained_draft", model, id } });
    if (response.kind !== "retained_draft") throw new Error("Unexpected sync response");
    return response.data;
  },
  async resumeObject(model: string, id: string): Promise<void> {
    const response = await invoke<SyncResponse>("che_model_sync", { request: { action: "resume_object", model, id } });
    if (response.kind !== "resolved") throw new Error("Unexpected sync response");
  },
  async discardRetainedDraft(model: string, id: string): Promise<void> {
    const response = await invoke<SyncResponse>("che_model_sync", { request: { action: "discard_retained_draft", model, id } });
    if (response.kind !== "resolved") throw new Error("Unexpected sync response");
  },
  async conflicts(): Promise<SyncConflict[]> {
    const response = await invoke<SyncResponse>("che_model_sync", {
      request: { action: "conflicts" },
    });
    if (response.kind !== "conflicts") throw new Error("Unexpected sync response");
    return response.data;
  },
  async resolve(model: string, id: string, resolution: SyncResolution): Promise<void> {
    const response = await invoke<SyncResponse>("che_model_sync", {
      request: { action: "resolve", model, id, resolution },
    });
    if (response.kind !== "resolved") throw new Error("Unexpected sync response");
  },
};
