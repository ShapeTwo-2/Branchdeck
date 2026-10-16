import { useEffect, useMemo, useRef, useState } from "react";
import { gitBranches, recipePlan, recipeRun } from "../lib/ipc";
import type { InputDef, InputValues, Plan, Recipe } from "../lib/types";

interface Props {
  recipe: Recipe;
  onClose: () => void;
}

function initialValues(inputs: InputDef[]): InputValues {
  const v: InputValues = {};
  for (const i of inputs) {
    if (i.type === "bool") v[i.id] = typeof i.default === "boolean" ? i.default : false;
    else if (i.type === "select") v[i.id] = typeof i.default === "string" ? i.default : (i.options?.[0] ?? "");
    else v[i.id] = typeof i.default === "string" ? i.default : "";
  }
  return v;
}

export function RecipeDialog({ recipe, onClose }: Props) {
  const [values, setValues] = useState<InputValues>(() => initialValues(recipe.inputs));
  const [plan, setPlan] = useState<{ key: string; plan: Plan } | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [runError, setRunError] = useState<string | null>(null);
  const [touched, setTouched] = useState<Set<string>>(new Set());
  const [submitted, setSubmitted] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [starting, setStarting] = useState(false);
  const [branches, setBranches] = useState<string[]>([]);
  const firstField = useRef<HTMLInputElement | HTMLSelectElement | null>(null);
  const runButton = useRef<HTMLButtonElement>(null);

  const key = useMemo(() => JSON.stringify(values), [values]);

  // Suggestions for "switch to an existing branch" style inputs.
  const wantsBranches = recipe.inputs.some((i) => i.type === "branch" && i.exists !== false);
  useEffect(() => {
    if (wantsBranches) gitBranches().then(setBranches).catch(() => setBranches([]));
  }, [wantsBranches]);

  // The backend renders the commands; refresh the preview as the user types.
  useEffect(() => {
    let cancelled = false;
    const t = setTimeout(() => {
      recipePlan(recipe.id, values)
        .then((p) => {
          if (!cancelled) {
            setPlan({ key, plan: p });
            setPlanError(null);
          }
        })
        .catch((e) => !cancelled && setPlanError(String(e)));
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
    // `key` stands in for `values` (same content, stable identity).
  }, [recipe.id, key]);

  useEffect(() => {
    (firstField.current ?? runButton.current)?.focus();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        if (confirming) setConfirming(false);
        else onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [confirming, onClose]);

  const current = plan?.key === key ? plan.plan : null;
  const fieldErrors = new Map<string, string>();
  const generalErrors: string[] = [];
  for (const e of current?.errors ?? []) {
    if (e.input) fieldErrors.set(e.input, e.message);
    else generalErrors.push(e.message);
  }
  const canRun = !!current && current.errors.length === 0 && current.steps.length > 0 && !starting;

  const set = (id: string, v: string | boolean) => {
    setValues((old) => ({ ...old, [id]: v }));
    setConfirming(false);
    setRunError(null);
  };

  const submit = async (e?: React.FormEvent) => {
    e?.preventDefault();
    setSubmitted(true);
    if (!canRun || !current) return;
    if (recipe.confirm && !confirming) {
      setConfirming(true);
      runButton.current?.focus();
      return;
    }
    setStarting(true);
    try {
      await recipeRun(
        recipe.id,
        values,
        current.steps.map((s) => s.command),
      );
      onClose();
    } catch (err) {
      setRunError(String(err));
      setConfirming(false);
    } finally {
      setStarting(false);
    }
  };

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form className="dialog" onSubmit={submit} aria-label={recipe.name}>
        <header className="dialog-header">
          <h2>{recipe.name}</h2>
          {recipe.description && <p className="muted">{recipe.description}</p>}
        </header>

        {recipe.inputs.length > 0 && (
          <div className="fields">
            {recipe.inputs.map((input, idx) => {
              const label = input.label ?? input.id;
              const err = fieldErrors.get(input.id);
              const showErr = err && (submitted || touched.has(input.id));
              const refProp = idx === 0 ? { ref: firstField as React.Ref<never> } : {};
              const blur = () => setTouched((t) => new Set(t).add(input.id));
              if (input.type === "bool") {
                return (
                  <label key={input.id} className="field field-bool">
                    <input
                      type="checkbox"
                      checked={values[input.id] === true}
                      onChange={(e) => set(input.id, e.target.checked)}
                      {...refProp}
                    />
                    <span>{label}</span>
                  </label>
                );
              }
              return (
                <label key={input.id} className="field">
                  <span className="field-label">
                    {label}
                    {input.required && <span className="req"> *</span>}
                  </span>
                  {input.type === "select" ? (
                    <select value={String(values[input.id])} onChange={(e) => set(input.id, e.target.value)} {...refProp}>
                      {input.options?.map((o) => (
                        <option key={o} value={o}>
                          {o}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      type="text"
                      list={input.type === "branch" && input.exists !== false ? "bd-branches" : undefined}
                      spellCheck={input.type !== "branch"}
                      autoComplete="off"
                      placeholder={input.placeholder ?? ""}
                      value={String(values[input.id] ?? "")}
                      onChange={(e) => set(input.id, e.target.value)}
                      onBlur={blur}
                      className={showErr ? "invalid" : ""}
                      {...refProp}
                    />
                  )}
                  {showErr && <span className="field-error">{err}</span>}
                </label>
              );
            })}
          </div>
        )}

        <div className="preview">
          <div className="preview-title">
            Commands that will be typed into the terminal
            {current && current.steps.length > 0 && <span className="muted"> ({current.steps.length})</span>}
          </div>
          {planError && <div className="error-box">{planError}</div>}
          <ol className="preview-steps">
            {(current ?? plan?.plan)?.steps.map((s, i) => (
              <li key={i} className={current ? "" : "stale"}>
                <code>{s.command}</code>
                {s.always && (
                  <span className="tag" title="Runs even if an earlier step fails">
                    always
                  </span>
                )}
              </li>
            ))}
          </ol>
          {generalErrors.map((m, i) => (
            <div key={i} className="error-box">
              {m}
            </div>
          ))}
          {current?.warnings.map((w, i) => (
            <div key={i} className="warning-box">
              {w}
            </div>
          ))}
        </div>
        <datalist id="bd-branches">
          {branches.map((b) => (
            <option key={b} value={b} />
          ))}
        </datalist>

        {runError && <div className="error-box">{runError}</div>}
        {confirming && (
          <div className="confirm-box">
            This recipe asks for confirmation. Run the {current?.steps.length} commands above?
          </div>
        )}

        <footer className="dialog-footer">
          <span className="muted kbd-hint">Esc to {confirming ? "go back" : "close"}</span>
          <button type="button" className="btn" onClick={confirming ? () => setConfirming(false) : onClose}>
            {confirming ? "Back" : "Cancel"}
          </button>
          <button
            type="submit"
            ref={runButton}
            className={`btn btn-primary${confirming ? " btn-danger" : ""}`}
            disabled={!canRun && submitted}
          >
            {starting ? "Starting…" : confirming ? "Yes, run it" : "Run"}
          </button>
        </footer>
      </form>
    </div>
  );
}
