/**
 * BgwsPlay — choose how to play, before anything else.
 *
 * Two games live behind /bgws/play, and they are different games:
 *
 *   Turn-based  the rulebook's game. Plan a turn, fight it activation by
 *               activation. Everything in TurnPlay.tsx.
 *   Real-time   a variant. Place units, give initial orders, press play:
 *               everything moves at once on one clock, and Jev decides what
 *               each unit does whenever something happens to it. Everything
 *               in realtime/, sharing no screen code with the turn game.
 *
 * ⚠ A SPLASH, NOT A TOGGLE INSIDE EITHER SCREEN. The two modes share rules
 * (fire tables, sighting, terrain) and nothing else, so neither screen knows
 * the other exists — which is what keeps the turn game unchanged. (They do
 * share the setup panel's look, from components/setupUi.)
 *
 * The choice is kept in the URL (?mode=turn / ?mode=realtime), so a refresh
 * keeps you where you were and the browser's back button returns here.
 */

import type { CSSProperties } from "react";
import { Link, useSearchParams } from "react-router-dom";

import TurnPlay from "./TurnPlay";
import RealtimePlay from "./realtime/RealtimePlay";
import "./setup.css";

type Mode = "turn" | "realtime";

const MODES: {
  mode: Mode;
  title: string;
  blurb: string;
  detail: string;
  steps: string[];
  glyph: string;
  accent: string;
}[] = [
  {
    mode: "turn",
    title: "Turn-based",
    blurb: "The rulebook's game",
    detail:
      "Plan each turn, then fight it one activation at a time with fog of war. " +
      "Scripted, AI-model or Jev-assisted commanders.",
    steps: ["Place forces", "Choose commanders", "Pick ground & rules", "Plan and fight turn by turn"],
    glyph: "▦",
    accent: "#7fbf7f",
  },
  {
    mode: "realtime",
    title: "Real-time",
    blurb: "Everything moves at once",
    detail:
      "Every unit moves, looks and shoots on one clock, and Jev decides what " +
      "each one does whenever something happens to it.",
    steps: ["Place units", "Choose who decides", "Set ground, comms & victory", "Review orders, press play"],
    glyph: "▶",
    accent: "#e8c547",
  },
];

export default function BgwsPlay() {
  const [params, setParams] = useSearchParams();
  const mode = params.get("mode");

  if (mode === "turn") return <TurnPlay />;
  if (mode === "realtime") return <RealtimePlay />;

  return (
    <div className="su-modes">
      <div className="su-modes-top">
        <Link to="/bgws" className="su-modes-back">
          &lsaquo; Main menu
        </Link>
      </div>

      <main className="su-modes-main">
        <header className="su-modes-head">
          <h1 className="su-modes-title">Choose how to play</h1>
          <p className="su-modes-sub">
            Both modes use the same forces, fire tables and terrain. You can switch any time from here.
          </p>
        </header>

        <div className="su-modes-grid">
          {MODES.map((entry) => (
            <button
              key={entry.mode}
              className="su-mode"
              style={{ "--mode-accent": entry.accent } as CSSProperties}
              onClick={() => setParams({ mode: entry.mode })}
            >
              <span className="su-mode-glyph" aria-hidden>
                {entry.glyph}
              </span>
              <span className="su-mode-title">{entry.title}</span>
              <span className="su-mode-blurb">{entry.blurb}</span>
              <span className="su-mode-detail">{entry.detail}</span>
              <ol className="su-mode-steps">
                {entry.steps.map((step) => (
                  <li key={step}>{step}</li>
                ))}
              </ol>
              <span className="su-mode-cta">Set up a game &rsaquo;</span>
            </button>
          ))}
        </div>
      </main>
    </div>
  );
}
