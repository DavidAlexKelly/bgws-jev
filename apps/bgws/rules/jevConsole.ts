// ── bgws/rules/jevConsole.ts ───────────────────────────────────────────────
// Every Jev decision, printed to the browser console as it is made.
//
// One collapsed group per decision: the headline says who, what and how sure;
// opening it shows the probabilities, the reasoning and the exact state that
// was sent. That last part is the point — the fastest way to find out why a
// tank held fire is to read what it was told, and the log keeps only the
// answer.

import type { Side } from "../lib/state";
import type { TacticalTrace } from "./tactical";

const STYLE: Record<Side, string> = {
  blue: "color:#7aa2e0;font-weight:600",
  red: "color:#e07a5f;font-weight:600",
};

/** One line that says what was decided, readable without opening anything. */
export function headline(side: Side, trace: TacticalTrace): string {
  const p = trace.probabilities?.[trace.chosenId];
  const who =
    trace.chosenBy === "jev"
      ? p != null
        ? `Jev ${Math.round(p * 100)}%`
        : "Jev"
      : trace.chosenBy === "llm"
        ? "escalated to commander"
        : `rules (Jev ${trace.fallback ?? "not asked"})`;
  return (
    `[Jev ${side}] ${trace.actorId ? `${trace.actorId} · ` : ""}${trace.question} ` +
    `→ ${trace.chosenId}  (${who}` +
    (trace.confidence != null ? `, confidence ${trace.confidence.toFixed(2)}` : "") +
    (trace.latencyMs ? `, ${trace.latencyMs} ms` : "") +
    ")"
  );
}

/** Print a decision. Never throws: a console that misbehaves must not stop a turn. */
export function printDecision(side: Side, trace: TacticalTrace, stateSent?: unknown): void {
  try {
    if (typeof console === "undefined") return;
    const line = headline(side, trace);
    if (typeof console.groupCollapsed !== "function") {
      console.log(line);
      return;
    }
    console.groupCollapsed(`%c${line}`, STYLE[side]);
    if (trace.probabilities) console.table?.(trace.probabilities);
    if (trace.rationale) console.log("why:", trace.rationale);
    console.log(
      "options:",
      trace.options.map((option) => `${option.id} — ${option.summary}`),
    );
    if (stateSent !== undefined) console.log("state sent to Jev:", stateSent);
    console.groupEnd();
  } catch {
    // Printing is a courtesy, never a dependency.
  }
}
