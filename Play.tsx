/**
 * BgwsPlay — choose how to play, before anything else.
 *
 * Two games live behind /bgws/play, and they are different games:
 *
 *   Turn-based  the rulebook's game. Plan a turn, fight it activation by
 *               activation. Everything in TurnPlay.tsx, exactly as it was.
 *   Real-time   a variant. Place units, give initial orders, press play:
 *               everything moves at once on one clock, and Jev decides what
 *               each unit does whenever something happens to it. Everything
 *               in realtime/, sharing no screen code with the turn game.
 *
 * ⚠ A SPLASH, NOT A TOGGLE INSIDE EITHER SCREEN. The two modes share rules
 * (fire tables, sighting, terrain) and nothing else, so neither screen knows
 * the other exists — which is what keeps the turn game unchanged.
 *
 * The choice is kept in the URL (?mode=turn / ?mode=realtime), so a refresh
 * keeps you where you were and the browser's back button returns here.
 */

import { Link, useSearchParams } from "react-router-dom";

import { AppSwitcher } from "@/components/AppSwitcher";

import TurnPlay from "./TurnPlay";
import RealtimePlay from "./realtime/RealtimePlay";

type Mode = "turn" | "realtime";

const MODES: { mode: Mode; title: string; blurb: string; detail: string; glyph: string; accent: string }[] = [
  {
    mode: "turn",
    title: "Turn-based",
    blurb: "The rulebook's game",
    detail:
      "Plan each turn, then fight it one activation at a time with fog of war. " +
      "Heuristic, language-model or Jev-assisted commanders.",
    glyph: "▦",
    accent: "#5a8f5a",
  },
  {
    mode: "realtime",
    title: "Real-time",
    blurb: "Everything moves at once",
    detail:
      "Place your units, give initial orders and press play. Every unit moves, " +
      "looks and shoots on one clock, and Jev decides what each one does " +
      "whenever something happens to it.",
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
    <div style={page}>
      <div style={header}>
        <Link to="/bgws" style={{ ...subtle, color: "#e8c547", textDecoration: "none" }}>
          &larr; BGWS
        </Link>
        <span style={{ fontWeight: 700, letterSpacing: "0.12em" }}>PLAY</span>
        <span style={subtle}>choose a mode</span>
        <span style={{ flex: 1 }} />
        <AppSwitcher />
      </div>

      <div style={grid}>
        {MODES.map((entry) => (
          <button
            key={entry.mode}
            onClick={() => setParams({ mode: entry.mode })}
            style={{ ...card, borderColor: "#191e37" }}
          >
            <div style={{ fontSize: 34, color: entry.accent, lineHeight: 1 }}>{entry.glyph}</div>
            <div style={{ fontSize: 18, fontWeight: 700, color: "#e9ecfb", marginTop: 14 }}>
              {entry.title}
            </div>
            <div style={{ ...subtle, color: entry.accent, marginTop: 2 }}>{entry.blurb}</div>
            <div style={{ ...subtle, marginTop: 10, lineHeight: 1.6 }}>{entry.detail}</div>
          </button>
        ))}
      </div>
    </div>
  );
}

const page: React.CSSProperties = {
  height: "100vh",
  background: "#060d18",
  color: "#e9ecfb",
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
  borderBottom: "1px solid #191e37",
  font: "11px/1 var(--font-mono, monospace)",
};

const grid: React.CSSProperties = {
  flex: 1,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  gap: 18,
  padding: 24,
  flexWrap: "wrap",
};

const card: React.CSSProperties = {
  width: 320,
  minHeight: 220,
  padding: 22,
  background: "rgba(255,255,255,0.02)",
  border: "1px solid",
  borderRadius: 8,
  cursor: "pointer",
  textAlign: "left",
  font: "inherit",
  color: "inherit",
  display: "flex",
  flexDirection: "column",
};

const subtle: React.CSSProperties = {
  fontSize: 11,
  color: "#6a7292",
};
