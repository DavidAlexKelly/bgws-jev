/**
 * BgwsHome — the way in.
 *
 * Two things live under /bgws now and they want different frames of mind: a
 * wargame you play, and a catalogue you browse. A chooser costs one click and
 * saves the second one from being discovered by accident.
 */

import { Link } from "react-router-dom";

import { AppSwitcher } from "@/components/AppSwitcher";

interface Section {
  to: string;
  title: string;
  blurb: string;
  detail: string;
  glyph: string;
  accent: string;
}

const SECTIONS: Section[] = [
  {
    to: "/bgws/play",
    title: "Play",
    blurb: "Fight an engagement",
    detail:
      "Pick a force list, a ground seed and which rules are in play, then watch the " +
      "engagement turn by turn with fog of war from either side.",
    glyph: "\u25b6",
    accent: "#5a8f5a",
  },
  {
    to: "/bgws/board",
    title: "Board",
    blurb: "Place counters by hand",
    detail:
      "A 10 × 10 km sheet on real terrain with a 1 km grid. Place force elements, " +
      "see what each side can see, and trace sight lines.",
    glyph: "▦",
    accent: "#e8c547",
  },
  {
    to: "/bgws/trial",
    title: "Trial",
    blurb: "Model against the bot",
    detail:
      "Run a batch of games with a language model commanding one side and the " +
      "scripted commander the other, and get the result as JSON. No map, no " +
      "turns to click through.",
    glyph: "\u2696",
    accent: "#b07acc",
  },
  {
    to: "/bgws/assets",
    title: "Asset explorer",
    blurb: "Browse the equipment",
    detail:
      "Every asset in the catalogue with its stats and photograph — mass, armour, " +
      "armament, ranges — and where each figure came from.",
    glyph: "⛭",
    accent: "#4A90D9",
  },
];

export default function BgwsHome() {
  return (
    <div style={page}>
      <div style={header}>
        <span style={{ fontWeight: 700, letterSpacing: "0.12em" }}>BGWS</span>
        <span style={subtle}>Battlegroup wargame · asset catalogue</span>
        <span style={{ flex: 1 }} />
        <AppSwitcher />
      </div>

      <div style={grid}>
        {SECTIONS.map((section) => (
          <Link key={section.to} to={section.to} style={{ ...card, borderColor: "#191e37" }}>
            <div style={{ fontSize: 34, color: section.accent, lineHeight: 1 }}>
              {section.glyph}
            </div>
            <div style={{ fontSize: 18, fontWeight: 700, color: "#e9ecfb", marginTop: 14 }}>
              {section.title}
            </div>
            <div style={{ ...subtle, color: section.accent, marginTop: 2 }}>{section.blurb}</div>
            <div style={{ ...subtle, marginTop: 10, lineHeight: 1.6 }}>{section.detail}</div>
          </Link>
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
  width: 300,
  minHeight: 220,
  padding: 22,
  background: "rgba(255,255,255,0.02)",
  border: "1px solid",
  borderRadius: 8,
  textDecoration: "none",
  display: "flex",
  flexDirection: "column",
};

const subtle: React.CSSProperties = {
  fontSize: 11,
  color: "#6a7292",
};
