import { useEffect } from "react";
import { recipeCancel } from "../lib/ipc";
import type { PlannedStep, RunDone, StepState } from "../lib/types";

export interface RunView {
  name: string;
  steps: PlannedStep[];
  states: StepState[];
  exitCodes: (number | null)[];
  notes: (string | null)[];
  done: RunDone | null;
}

const ICON: Record<StepState, string> = {
  pending: "○",
  running: "◐",
  ok: "✓",
  failed: "✗",
  skipped: "–",
};

interface Props {
  run: RunView;
  onDismiss: () => void;
}

export function RunPanel({ run, onDismiss }: Props) {
  const { done } = run;

  // Successful runs get out of the way on their own.
  useEffect(() => {
    if (done?.status !== "ok") return;
    const t = setTimeout(onDismiss, 4000);
    return () => clearTimeout(t);
  }, [done, onDismiss]);

  let title: string;
  let tone: string;
  if (!done) {
    title = `Running: ${run.name}`;
    tone = "running";
  } else if (done.status === "ok") {
    title = `Done: ${run.name}`;
    tone = "ok";
  } else if (done.status === "cancelled") {
    title = `Cancelled: ${run.name}`;
    tone = "warn";
  } else if (done.failedStep !== null) {
    title = `${run.name} stopped: step ${done.failedStep + 1} failed (exit ${done.exitCode})`;
    tone = "failed";
  } else {
    title = `${run.name} stopped`;
    tone = "failed";
  }

  return (
    <section className={`run-panel run-${tone}`} aria-live="polite">
      <header className="run-header">
        <span className="run-title">{title}</span>
        {!done ? (
          <button className="btn btn-small" onClick={() => void recipeCancel()} title="Sends Ctrl+C to the running step">
            Stop
          </button>
        ) : (
          <button className="btn btn-small" onClick={onDismiss}>
            Dismiss
          </button>
        )}
      </header>
      <ol className="run-steps">
        {run.steps.map((s, i) => (
          <li key={i} className={`step step-${run.states[i]}`}>
            <span className="step-icon">{ICON[run.states[i]]}</span>
            <code>{s.command}</code>
            {run.states[i] === "failed" && run.exitCodes[i] !== null && (
              <span className="step-note">exit {run.exitCodes[i]}</span>
            )}
            {run.notes[i] && <span className="step-note">{run.notes[i]}</span>}
          </li>
        ))}
      </ol>
      {done?.message && <div className="run-message">{done.message}</div>}
      {done?.operation && (
        <div className="run-message warn">
          Git is in the middle of a {done.operation}. Resolve the conflicts in the terminal and continue, or run{" "}
          <code>git {done.operation} --abort</code>. (One-click Resolve / Abort buttons come with the safety milestone.)
        </div>
      )}
    </section>
  );
}
