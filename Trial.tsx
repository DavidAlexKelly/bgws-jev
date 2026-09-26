/**
 * TRIAL — run the model against the bot, in the browser, and hand back JSON.
 *
 * ⚠ THE BROWSER IS THE ONLY PLACE THAT HAS BOTH HALVES, and it took an
 * embarrassingly long time to notice.
 *
 * A trial needs the rules and a model. The rules are TypeScript and run
 * anywhere. The model is only reachable through the published commanderTurn
 * query, and a Code Workspace container cannot call it — its egress proxy
 * disables executeQueryV2. That was repeatedly described as "nowhere has
 * both", and the conclusion drawn was that somebody had to run a script on a
 * laptop with a token.
 *
 * Wrong. THIS page has both: the engine is bundled into it, and the OSDK
 * client authenticates as whoever is signed in, which is exactly how the Play
 * screen already calls a model every turn. No token, no laptop, no egress.
 *
 * So this is the Play screen with the game taken out: no map, no counters, no
 * turn-by-turn. Pick a challenger, press run, wait, copy the JSON.
 *
 * WHAT IT COSTS, because it is easy to start something expensive here.
 * Every seed is played TWICE (challenger as blue, then as red — see
 * commanderTrial.ts for why), and each turn costs the challenger two model
 * calls: one for its orders and one for the counteraction round. So:
 *
 *     calls ~= seeds x 2 games x turns x 2
 *
 * Three seeds over eight turns is about 96 calls and takes minutes, not
 * seconds. The estimate is shown before anything is spent.
 */

import { useCallback, useMemo, useRef, useState } from "react";

import { AppSwitcher } from "@/components/AppSwitcher";

import { COMMANDER_MODELS, foundryModelCall, type CommanderModelName } from "./data/commanderClient";
import { jevConfigured, openRouterJevCall } from "./data/jevClient";
import { withPersistentCache } from "./data/jevCache";
import { JEV_MODEL } from "./rules/jev";
import { jevTacticalDecider } from "./rules/jevDecider";
import { scenarioFactory } from "./lib/forceBuilder";
import { proceduralTerrain, STANDARD_GROUND } from "./lib/proceduralTerrain";
import type { Side } from "./lib/state";
import { describeTrial, runTrial, type TrialResult } from "./rules/commanderTrial";
import { FORCE_LISTS, SYMMETRIC_CONTROL_V1 } from "./rules/forceList";
import { llmCommander } from "./rules/llmCommander";
import { heuristicOrdersCommander } from "./rules/orders";
import { HOUSE_V1, withModules } from "./rules/ruleset";

/**
 * Everything on. A commander should be judged on the whole game, not on the
 * subset of it the default ruleset happens to switch on.
 */
const RULES = withModules(HOUSE_V1, {
  reactionFire: true,
  counteraction: true,
  defensiveFire: true,
  closeCombat: true,
  combinedFire: true,
  commandActivations: true,
  ammunition: true,
  indirectFire: true,
});

const terrain = proceduralTerrain(STANDARD_GROUND);

export default function BgwsTrial() {
  // "heuristic" as a challenger is only interesting with Jev on: it isolates
  // Jev's in-the-moment calls against the same planner on the other side.
  const [model, setModel] = useState<CommanderModelName | "heuristic">(COMMANDER_MODELS[0]);
  const [useJev, setUseJev] = useState(false);
  const [listId, setListId] = useState(SYMMETRIC_CONTROL_V1.id);
  const [seeds, setSeeds] = useState(3);
  const [maxTurns, setMaxTurns] = useState(8);
  const [directive, setDirective] = useState("");

  const [running, setRunning] = useState(false);
  const [progress, setProgress] = useState<string[]>([]);
  const [output, setOutput] = useState("");
  const [error, setError] = useState<string | null>(null);
  const outputRef = useRef<HTMLTextAreaElement>(null);

  const estimatedCalls = model === "heuristic" ? 0 : seeds * 2 * maxTurns * 2;

  const run = useCallback(async () => {
    setRunning(true);
    setError(null);
    setOutput("");
    setProgress([`starting: ${seeds * 2} games, up to ${maxTurns} turns each`]);

    // Persistent: re-running a trial asks Jev nothing it has been asked before,
    // so a re-run reproduces the first run's decisions and costs nothing.
    const jevCall = useJev
      ? withPersistentCache(openRouterJevCall(), { namespace: JEV_MODEL })
      : undefined;
    const label = `${model}${useJev ? " + Jev" : ""}`;

    try {
      const result: TrialResult = await runTrial({
        ruleset: RULES,
        terrain,
        scenario: scenarioFactory(FORCE_LISTS[listId], RULES),
        seeds: Array.from({ length: seeds }, (_, index) => `trial${index}`),
        maxTurns,
        challenger: (side: Side) =>
          model === "heuristic"
            ? heuristicOrdersCommander(side, { coLocatedM: RULES.coLocatedM })
            : llmCommander({
                side,
                call: foundryModelCall(model, directive, useJev ? "turnJev" : "turn"),
                directive,
                name: `${model}-${side}`,
                jev: useJev,
              }),
        challengerTactics: jevCall
          ? (side: Side) =>
              jevTacticalDecider({
                side,
                call: jevCall,
                directive,
                escalate: model === "heuristic" ? undefined : foundryModelCall(model, "", "decide"),
                // A trial prints nothing per decision: hundreds of groups would
                // bury the progress lines. The JSON result has them all.
                log: false,
              })
          : undefined,
        challengerPositions: useJev,
        // The baseline keeps formation, because combinedFire is on and a
        // commander that cannot mass is not the yardstick anybody wants.
        baseline: (side: Side) =>
          heuristicOrdersCommander(side, { coLocatedM: RULES.coLocatedM }),
        onProgress: (finished, total, game) => {
          setProgress((lines) => [
            ...lines,
            `${finished}/${total}  seed ${game.seed} as ${game.challengerSide}: ` +
              `${game.winner ?? "drawn"} in ${game.turns} turns, ` +
              `${game.ordersRejected}/${game.ordersIssued} orders refused` +
              (game.failedTurns > 0 ? `, ${game.failedTurns} turns unanswered` : "") +
              (game.tacticalCalls > 0
                ? `, ${game.tacticalCalls} Jev calls (${game.tacticalFallbacks} fell back)`
                : ""),
          ]);
        },
      });

      // The summary first, because it is the part that refuses to give a
      // verdict when the model did not answer — see describeTrial.
      setOutput(
        [
          describeTrial(result, `${label} vs heuristic on ${listId}`),
          "",
          JSON.stringify(
            {
              model,
              jev: useJev,
              directive: directive.trim() || null,
              forceList: listId,
              ruleset: RULES.id,
              maxTurns,
              result,
            },
            null,
            2,
          ),
        ].join("\n"),
      );
    } catch (thrown) {
      setError(thrown instanceof Error ? thrown.message : String(thrown));
    } finally {
      setRunning(false);
    }
  }, [model, useJev, listId, seeds, maxTurns, directive]);

  const copy = useCallback(() => {
    // Selecting as well as copying: if the clipboard API is refused — it is,
    // in some embedded contexts — the text is at least highlighted ready for
    // a manual copy, rather than the button appearing to do nothing.
    outputRef.current?.select();
    void navigator.clipboard?.writeText(output);
  }, [output]);

  const lists = useMemo(() => Object.values(FORCE_LISTS), []);

  return (
    <div style={page}>
      <div style={header}>
        <span style={{ fontWeight: 700, letterSpacing: "0.12em" }}>BGWS TRIAL</span>
        <span style={subtle}>model against the scripted commander</span>
        <span style={{ flex: 1 }} />
        <AppSwitcher />
      </div>

      <div style={body}>
        <div style={panel}>
          <div style={groupTitle}>Challenger</div>
          <select
            value={model}
            onChange={(event) => setModel(event.target.value as CommanderModelName | "heuristic")}
            style={field}
            disabled={running}
          >
            <option value="heuristic">heuristic</option>
            {COMMANDER_MODELS.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>

          <label style={{ ...note, display: "flex", alignItems: "center", gap: 6 }}>
            <input
              type="checkbox"
              checked={useJev}
              onChange={(event) => setUseJev(event.target.checked)}
              disabled={running}
            />
            Use Jev for decisions (challenger&rsquo;s side)
          </label>
          <div style={note}>
            Jev makes the challenger&rsquo;s in-the-moment calls — which ordered
            unit acts next, reactive fire, contact, spotting, reserve follow-ups —
            and the challenger alone gets terrain-aware moves. The baseline plays
            by the rules. Heuristic + Jev against heuristic measures Jev alone.
            {useJev && !jevConfigured() && (
              <strong> No OpenRouter key: every call will fall back.</strong>
            )}
          </div>

          <div style={groupTitle}>Force list</div>
          <select
            value={listId}
            onChange={(event) => setListId(event.target.value)}
            style={field}
            disabled={running}
          >
            {lists.map((list) => (
              <option key={list.id} value={list.id}>
                {list.name}
              </option>
            ))}
          </select>
          <div style={note}>
            Symmetric control is the fairest: identical forces, so a win is
            attributable to command rather than to equipment.
          </div>

          <div style={groupTitle}>Seeds (each played twice)</div>
          <input
            type="number"
            min={1}
            max={10}
            value={seeds}
            onChange={(event) => setSeeds(Math.max(1, Math.min(10, Number(event.target.value))))}
            style={field}
            disabled={running}
          />

          <div style={groupTitle}>Turn cap</div>
          <input
            type="number"
            min={3}
            max={40}
            value={maxTurns}
            onChange={(event) => setMaxTurns(Math.max(3, Math.min(40, Number(event.target.value))))}
            style={field}
            disabled={running}
          />
          <div style={note}>
            A capped game that reaches the limit is decided on remaining combat
            strength, not called a draw.
          </div>

          <div style={groupTitle}>Directive (optional)</div>
          <textarea
            value={directive}
            onChange={(event) => setDirective(event.target.value)}
            rows={3}
            style={{ ...field, resize: "vertical" }}
            disabled={running}
            placeholder="e.g. hold the ridge; do not accept losses to take ground"
          />

          <div style={{ ...note, marginTop: 10 }}>
            About <strong>{estimatedCalls}</strong> commander model calls
            {" "}({seeds * 2} games &times; {maxTurns} turns &times; 2 calls a turn).
            Sequential on purpose — firing them all at once is how a trial
            becomes a rate-limit incident.
          </div>

          <button onClick={run} disabled={running} style={runButton}>
            {running ? "running…" : "Run trial"}
          </button>
        </div>

        <div style={results}>
          {error && <div style={errorBox}>{error}</div>}

          {progress.length > 0 && (
            <div style={progressBox}>
              {progress.map((line, index) => (
                <div key={index}>{line}</div>
              ))}
            </div>
          )}

          {output && (
            <>
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
                <span style={groupTitle}>Result</span>
                <button onClick={copy} style={copyButton}>
                  copy
                </button>
              </div>
              <textarea ref={outputRef} value={output} readOnly style={outputBox} />
            </>
          )}

          {!output && !running && progress.length === 0 && (
            <div style={note}>
              Nothing run yet. This page exists so a trial can be run without a
              laptop, a token or the map — the browser is the only place that
              has both the rules and a reachable model.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

const page: React.CSSProperties = {
  height: "100vh",
  background: "#060d18",
  color: "#d4d9e8",
  font: "12px/1.5 var(--font-mono, monospace)",
  display: "flex",
  flexDirection: "column",
};

const header: React.CSSProperties = {
  height: 40,
  display: "flex",
  alignItems: "center",
  gap: 12,
  padding: "0 12px",
  borderBottom: "1px solid #1e2330",
  font: "11px/1 var(--font-mono, monospace)",
};

const body: React.CSSProperties = { flex: 1, display: "flex", minHeight: 0 };

const panel: React.CSSProperties = {
  width: 320,
  padding: 14,
  borderRight: "1px solid #1e2330",
  overflowY: "auto",
};

const results: React.CSSProperties = {
  flex: 1,
  padding: 14,
  display: "flex",
  flexDirection: "column",
  minHeight: 0,
};

const groupTitle: React.CSSProperties = {
  fontSize: 11,
  color: "#8b93a8",
  letterSpacing: "0.08em",
  textTransform: "uppercase",
  marginTop: 12,
  marginBottom: 4,
};

const field: React.CSSProperties = {
  width: "100%",
  background: "rgba(255,255,255,0.03)",
  border: "1px solid #1e2330",
  borderRadius: 4,
  color: "#d4d9e8",
  font: "12px/1.5 var(--font-mono, monospace)",
  padding: "5px 7px",
};

const note: React.CSSProperties = { fontSize: 11, color: "#5a6178", lineHeight: 1.6 };

const runButton: React.CSSProperties = {
  ...field,
  marginTop: 12,
  cursor: "pointer",
  background: "#1b3a5c",
  borderColor: "#2b5580",
  fontWeight: 700,
};

const copyButton: React.CSSProperties = {
  background: "rgba(255,255,255,0.03)",
  border: "1px solid #1e2330",
  borderRadius: 4,
  color: "#8b93a8",
  font: "11px var(--font-mono, monospace)",
  padding: "2px 8px",
  cursor: "pointer",
};

const progressBox: React.CSSProperties = {
  border: "1px solid #1e2330",
  borderRadius: 4,
  padding: 8,
  marginBottom: 10,
  maxHeight: 160,
  overflowY: "auto",
  fontSize: 11,
  color: "#8b93a8",
};

const outputBox: React.CSSProperties = {
  flex: 1,
  width: "100%",
  background: "rgba(255,255,255,0.02)",
  border: "1px solid #1e2330",
  borderRadius: 4,
  color: "#d4d9e8",
  font: "11px/1.5 var(--font-mono, monospace)",
  padding: 8,
  resize: "none",
};

const errorBox: React.CSSProperties = {
  border: "1px solid #5c1b1b",
  background: "rgba(224,122,95,0.08)",
  borderRadius: 4,
  padding: 8,
  marginBottom: 10,
  color: "#e07a5f",
  fontSize: 11,
};

const subtle: React.CSSProperties = { fontSize: 11, color: "#5a6178" };
