// Mirrors of the Rust structs in src-tauri/src. Keep field names in sync (serde uses camelCase).

export interface SpawnInfo {
  /** Absolute path of the shell binary that was started. */
  shell: string;
  args: string[];
  cwd: string;
  /** The prompt hook is installed, so recipes can run. */
  integration: boolean;
  /** Present on Windows: tells xterm.js it is talking to ConPTY. */
  windowsBuild: number | null;
}

export interface PtyExit {
  code: number | null;
}

export interface PromptEvent {
  exitCode: number;
  cwd: string;
}

export type InputKind = "text" | "branch" | "select" | "bool";

export interface InputDef {
  id: string;
  label: string | null;
  placeholder: string | null;
  required: boolean;
  type: InputKind;
  options?: string[];
  default: string | boolean | null;
}

export type RecipeSource = "builtin" | "global" | "repo";

export interface Recipe {
  id: string;
  name: string;
  description: string | null;
  source: RecipeSource;
  confirm: boolean;
  inputs: InputDef[];
}

export interface LoadError {
  file: string;
  message: string;
}

export interface RecipeList {
  cwd: string | null;
  repoRoot: string | null;
  recipes: Recipe[];
  errors: LoadError[];
  globalFile: string | null;
}

export type InputValues = Record<string, string | boolean>;

export interface PlannedStep {
  command: string;
  always: boolean;
}

export interface FieldError {
  input: string | null;
  message: string;
}

export interface Plan {
  steps: PlannedStep[];
  errors: FieldError[];
}

export type StepState = "pending" | "running" | "ok" | "failed" | "skipped";
export type GitOperation = "merge" | "rebase" | "cherry-pick" | "revert";

export interface RunStarted {
  name: string;
  steps: PlannedStep[];
}

export interface StepEvent {
  index: number;
  state: StepState;
  exitCode: number | null;
  note: string | null;
}

export interface RunDone {
  status: "ok" | "failed" | "cancelled";
  failedStep: number | null;
  exitCode: number | null;
  operation: GitOperation | null;
  message: string | null;
}
