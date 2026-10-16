// Typed wrappers around every Tauri command and event the frontend uses.
// Components never call `invoke` / `listen` directly.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { PtyExit, SpawnInfo } from "./types";

export function ptySpawn(cols: number, rows: number, onData: (chunk: string) => void): Promise<SpawnInfo> {
  const channel = new Channel<string>();
  channel.onmessage = onData;
  return invoke<SpawnInfo>("pty_spawn", { cols, rows, onData: channel });
}

export function ptyWrite(data: string): Promise<void> {
  return invoke("pty_write", { data });
}

export function ptyResize(cols: number, rows: number): Promise<void> {
  return invoke("pty_resize", { cols, rows });
}

export function onPtyExit(handler: (exit: PtyExit) => void): Promise<UnlistenFn> {
  return listen<PtyExit>("pty://exit", (e) => handler(e.payload));
}
