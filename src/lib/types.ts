// Mirrors of the Rust structs in src-tauri/src. Keep field names in sync (serde uses camelCase).

export interface SpawnInfo {
  /** Absolute path of the shell binary that was started. */
  shell: string;
  args: string[];
  cwd: string;
  /** Present on Windows: tells xterm.js it is talking to ConPTY. */
  windowsBuild: number | null;
}

export interface PtyExit {
  code: number | null;
}
