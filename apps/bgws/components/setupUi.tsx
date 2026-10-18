/**
 * The guided setup panel both play modes use, and its controls.
 *
 * Presentation only. Each screen keeps its own setup state exactly where it
 * was; these components take values and callbacks, so the rules about what a
 * choice means (and every explanation of it) stay with the screen that owns it.
 * Styling is in ../setup.css (`su-` classes).
 */

import { useId, type ReactNode } from "react";

// ── The panel ──────────────────────────────────────────────────────────────

export interface SetupStep {
  id: string;
  label: string;
  /** Ticked in the step bar. */
  done?: boolean;
}

/**
 * The floating setup panel: a step bar, one step's content, and a footer of
 * Back / Next / Start.
 */
export function SetupPanel({
  kicker,
  title,
  steps,
  active,
  onStep,
  footer,
  children,
}: {
  kicker: string;
  title: string;
  steps: readonly SetupStep[];
  active: number;
  onStep: (index: number) => void;
  footer: ReactNode;
  children: ReactNode;
}) {
  return (
    <aside className="su-panel" aria-label="Game setup">
      <div className="su-head">
        <div className="su-head-text">
          <div className="su-kicker">
            {kicker} &middot; step {active + 1} of {steps.length}
          </div>
          <h2 className="su-title">{title}</h2>
        </div>
      </div>

      <nav className="su-steps" aria-label="Setup steps">
        {steps.map((step, index) => (
          <button
            key={step.id}
            className="su-step"
            aria-current={index === active ? "step" : undefined}
            data-done={step.done || undefined}
            onClick={() => onStep(index)}
          >
            <span className="su-step-dot" aria-hidden>
              {step.done && index !== active ? "✓" : index + 1}
            </span>
            {step.label}
          </button>
        ))}
      </nav>

      <div className="su-body">{children}</div>
      <div className="su-foot">{footer}</div>
    </aside>
  );
}

export function Section({ title, children }: { title?: string; children: ReactNode }) {
  return (
    <section className="su-section">
      {title && <h3 className="su-section-title">{title}</h3>}
      {children}
    </section>
  );
}

export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <label className="su-field">
      <span className="su-field-label">{label}</span>
      {children}
      {hint && <span className="su-hint">{hint}</span>}
    </label>
  );
}

// ── Choosing ───────────────────────────────────────────────────────────────

export interface SegmentOption<T extends string> {
  value: T;
  label: ReactNode;
  tone?: "blue" | "red";
}

/** A small set of mutually exclusive choices, shown all at once. */
export function Segmented<T extends string>({
  label,
  value,
  onChange,
  options,
  size,
}: {
  label: string;
  value: T;
  onChange: (value: T) => void;
  options: readonly SegmentOption<T>[];
  size?: "lg";
}) {
  return (
    <div className="su-seg" role="group" aria-label={label} data-size={size}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className="su-seg-btn"
          aria-pressed={option.value === value}
          data-tone={option.tone}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export interface ChoiceOption<T extends string> {
  value: T;
  title: string;
  description?: ReactNode;
  badge?: string;
}

/** Radio choices as cards, each saying what it means. */
export function ChoiceCards<T extends string>({
  label,
  value,
  onChange,
  options,
  columns,
}: {
  label: string;
  value: T;
  onChange: (value: T) => void;
  options: readonly ChoiceOption<T>[];
  columns?: 2;
}) {
  return (
    <div className="su-choices" role="radiogroup" aria-label={label} data-columns={columns}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          className="su-choice"
          aria-checked={option.value === value}
          onClick={() => onChange(option.value)}
        >
          <span className="su-choice-radio" aria-hidden />
          <span className="su-choice-text">
            <span className="su-choice-title">
              {option.title}
              {option.badge && <span className="su-badge">{option.badge}</span>}
            </span>
            {option.description && <span className="su-choice-desc">{option.description}</span>}
          </span>
        </button>
      ))}
    </div>
  );
}

/** − value + , clamped. Typing is still allowed. */
export function NumberStepper({
  label,
  value,
  min,
  max,
  step = 1,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
}) {
  const clamp = (n: number) => Math.max(min, Math.min(max, n));
  return (
    <div className="su-stepper" role="group" aria-label={label}>
      <button
        type="button"
        onClick={() => onChange(clamp(value - step))}
        disabled={value <= min}
        aria-label={`Fewer: ${label}`}
      >
        &minus;
      </button>
      <output aria-live="polite">{value}</output>
      <button
        type="button"
        onClick={() => onChange(clamp(value + step))}
        disabled={value >= max}
        aria-label={`More: ${label}`}
      >
        +
      </button>
    </div>
  );
}

/** An on/off setting with what it does underneath. */
export function Switch({
  checked,
  onChange,
  label,
  description,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <label className="su-switch" htmlFor={id} data-disabled={disabled || undefined}>
      <input
        id={id}
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      <span className="su-switch-track" aria-hidden />
      <span className="su-switch-text">
        <span className="su-switch-label">{label}</span>
        {description && <span className="su-switch-desc">{description}</span>}
      </span>
    </label>
  );
}

// ── Telling ────────────────────────────────────────────────────────────────

const CALLOUT_ICON = { info: "i", warn: "!", danger: "!", ok: "✓" } as const;

export function Callout({
  tone = "info",
  children,
}: {
  tone?: keyof typeof CALLOUT_ICON;
  children: ReactNode;
}) {
  return (
    <div className="su-callout" data-tone={tone} role={tone === "danger" ? "alert" : undefined}>
      <span className="su-callout-icon" aria-hidden>
        {CALLOUT_ICON[tone]}
      </span>
      <div>{children}</div>
    </div>
  );
}

/** Detail most people will not need, folded away rather than removed. */
export function Disclosure({
  summary,
  meta,
  children,
}: {
  summary: string;
  meta?: string;
  children: ReactNode;
}) {
  return (
    <details className="su-disclosure">
      <summary>
        {summary}
        {meta && <span className="su-disclosure-meta">{meta}</span>}
      </summary>
      <div className="su-disclosure-body">{children}</div>
    </details>
  );
}

/** What the next click on the board will do. */
export function PlacementHint({ side, text }: { side: "blue" | "red"; text: ReactNode }) {
  return (
    <div className="su-place" data-side={side}>
      <span className="su-place-icon" aria-hidden>
        ⌖
      </span>
      <div>{text}</div>
    </div>
  );
}

interface RosterSide {
  count: number;
  /** e.g. "CS 42". */
  detail?: string;
}

/** Both sides at a glance, as forces are placed. */
export function Roster({
  blue,
  red,
  unit = "element",
  onClear,
}: {
  blue: RosterSide;
  red: RosterSide;
  unit?: string;
  onClear?: () => void;
}) {
  const total = blue.count + red.count;
  return (
    <Section title="On the board">
      <div className="su-roster">
        {(
          [
            ["blue", "Blue", blue],
            ["red", "Red", red],
          ] as const
        ).map(([key, name, s]) => (
          <div key={key} className="su-roster-side" data-side={key}>
            <span className="su-roster-name">{name}</span>
            {s.count > 0 ? (
              <>
                <span className="su-roster-big">{s.count}</span>
                <span className="su-roster-sub">
                  {unit}
                  {s.count === 1 ? "" : "s"}
                  {s.detail ? ` · ${s.detail}` : ""}
                </span>
              </>
            ) : (
              <span className="su-roster-empty">Nothing placed yet</span>
            )}
          </div>
        ))}
      </div>
      {onClear && total > 0 && (
        <button className="su-btn" data-variant="ghost" data-size="sm" onClick={onClear}>
          Clear the board ({total})
        </button>
      )}
    </Section>
  );
}

/** A card per side, edged in the side's colour. */
export function SideCard({
  side,
  title,
  children,
}: {
  side: "blue" | "red";
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="su-side-card" data-side={side}>
      <div className="su-side-card-title">{title}</div>
      {children}
    </div>
  );
}

export interface ReviewRow {
  label: string;
  value: ReactNode;
  /** The step that sets this, for the Edit link. */
  step?: number;
}

/** What you are about to play, with a way back to change each part. */
export function Review({ rows, onEdit }: { rows: readonly ReviewRow[]; onEdit: (step: number) => void }) {
  return (
    <div className="su-review">
      {rows.map((row) => (
        <div key={row.label} className="su-review-row">
          <span className="su-review-label">{row.label}</span>
          <span className="su-review-value">{row.value}</span>
          {row.step != null && (
            <button className="su-link" onClick={() => onEdit(row.step as number)}>
              Edit
            </button>
          )}
        </div>
      ))}
    </div>
  );
}

/** Why the game cannot start yet, each with a way to fix it. */
export function Blockers({
  items,
}: {
  items: readonly { text: string; fix?: () => void; fixLabel?: string }[];
}) {
  if (items.length === 0) return null;
  return (
    <div className="su-blockers">
      {items.map((item) => (
        <Callout key={item.text} tone="warn">
          {item.text}{" "}
          {item.fix && (
            <button className="su-link" onClick={item.fix}>
              {item.fixLabel ?? "Fix"} &rsaquo;
            </button>
          )}
        </Callout>
      ))}
    </div>
  );
}
