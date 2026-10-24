// ── behaviour/types.ts ───────────────────────────────────────────────────
// The shape of a behaviour template: a unit type's decision graph, stored as
// a JSON file under ./templates (docs/plan/DECISION_FLOW_GRAPHS.md §3, §6).
//
// Templates are DATA ONLY. They name facts and actions; they never contain
// code. The engine evaluates the facts against the real state and walks the
// graph; Jev is asked only at a `choose` node, and only about options that
// survived their conditions. So a template can change what a unit is
// offered and how it is worded, but can never break determinism or offer
// Jev something illegal.

/** A comparison against one named fact (see FactTable). */
export type Comparison = "==" | "!=" | ">" | ">=" | "<" | "<=";
export const COMPARISONS: readonly Comparison[] = ["==", "!=", ">", ">=", "<", "<="];

export type FactValue = number | string | boolean | null;

export type Condition =
  | { fact: string; op: Comparison; value: FactValue }
  | { all: Condition[] }
  | { any: Condition[] }
  | { not: Condition };

/** Preference nudge Jev is told about. Never removes an option (`remove` does that). */
export type Bias = -2 | -1 | 0 | 1 | 2;

export interface OptionSpec {
  /** Unique within its node. What Jev answers with, and what traces record. */
  id: string;
  /** An action in the action library (actions/*.ts), referred to by name. */
  action: string;
  /** Offered only when this holds. Absent = always offered. */
  when?: Condition;
  bias?: Bias;
  /** Static parameters handed to the action. */
  params?: Record<string, FactValue>;
}

/** Ask Jev to pick one option. `fallback` is used when Jev cannot answer. */
export interface ChooseNode {
  kind: "choose";
  /** Short heading, sent to Jev as the decision's title. */
  title: string;
  /** The question itself, sent to Jev. */
  question: string;
  options: OptionSpec[];
  /** Option id taken when Jev is unavailable, times out, or picks nothing valid. */
  fallback: string;
}

/** Deterministic split: the first case whose condition holds; otherwise `else`. */
export interface BranchNode {
  kind: "branch";
  cases: { when: Condition; next: string }[];
  else: string;
}

/** A fixed response: no question asked (a drill). */
export interface DoNode {
  kind: "do";
  action: string;
  params?: Record<string, FactValue>;
}

export type GraphNode = ChooseNode | BranchNode | DoNode;

export interface Template {
  /** "core/tank", "custom/tank-stand-fast". Matches the file's path under templates/. */
  id: string;
  version: number;
  /** Display name, for the Behaviour Manager. Ignored by the engine. */
  name?: string;
  /** The unit archetype this is the default for, if any. */
  archetype?: string;
  /** Inherit triggers and nodes from another template; own entries override. */
  parent?: string;
  /** Plain-language doctrine, passed to Jev alongside every question. */
  doctrine?: string;
  /** Event kind → the node a decision starts at. */
  triggers?: Record<string, string>;
  nodes?: Record<string, GraphNode>;
  /** Option ids this template removes from every choose node (inherited ones too). */
  remove?: string[];
  /** Option id → bias override, applied across every choose node. */
  bias?: Record<string, Bias>;
  /**
   * Option id → extra condition, across every choose node: the option is
   * offered only if its own `when` AND this hold. How a variant adds "only
   * when…" to an option it inherits without redefining the node.
   */
  when?: Record<string, Condition>;
}

/** A template with its parent chain merged in. What the walker runs. */
export interface ResolvedTemplate {
  id: string;
  /** This template first, then each ancestor. */
  lineage: string[];
  doctrine: string[];
  triggers: Record<string, string>;
  nodes: Record<string, GraphNode>;
  remove: Set<string>;
  bias: Record<string, Bias>;
  when: Record<string, Condition>;
}

/**
 * The facts a template may name, supplied by the engine for the unit being
 * decided for. Kept as a table of functions so the walker stays generic and
 * the vocabulary is closed: a template naming a fact not in the table fails
 * validation, rather than silently evaluating to nothing.
 */
export type FactTable<Ctx> = Record<string, (ctx: Ctx) => FactValue>;

export type WalkResult =
  | {
      kind: "choose";
      nodeId: string;
      title: string;
      question: string;
      doctrine: string[];
      /** Options that survived their conditions and the template's removals, in order. */
      options: (OptionSpec & { bias: Bias })[];
      /** The node's fallback if it survived; otherwise the first surviving option. */
      fallback: string | null;
      /** The node's other options, and why each is not on offer: the template removed it, or its condition failed. */
      dropped: { id: string; action: string; why: "removed" | "condition" }[];
      /** Every node visited, trigger node first. */
      path: string[];
    }
  | { kind: "do"; nodeId: string; action: string; params: Record<string, FactValue>; path: string[] }
  | { kind: "none"; path: string[] };
