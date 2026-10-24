/**
 * AssetHub — the Asset Manager's front door: what the forces are made of,
 * and how they behave.
 */

import type { CSSProperties } from "react";
import { Link, useNavigate } from "react-router-dom";
import "./setup.css";

const SECTIONS = [
  {
    to: "/bgws/assets/units",
    title: "Unit & Equipment Manager",
    blurb: "Every platform and weapon, with its stats",
    detail:
      "Browse and filter the equipment catalogue and munitions, compare protection and reach, and see which behaviour each asset uses.",
    glyph: "▦",
    accent: "#4fc3f7",
    cta: "Open equipment",
  },
  {
    to: "/bgws/assets/behaviours",
    title: "Behaviour Manager",
    blurb: "How units decide what to do",
    detail:
      "Create, copy, edit and delete behaviour templates: which options a unit is offered at each decision, and how Jev is nudged to choose between them.",
    glyph: "⚙",
    accent: "#9fd8a9",
    cta: "Open behaviours",
  },
] as const;

export default function AssetHub() {
  const navigate = useNavigate();
  return (
    <div className="su-modes">
      <div className="su-modes-top">
        <Link to="/bgws" className="su-modes-back">
          ‹ Main menu
        </Link>
      </div>
      <main className="su-modes-main">
        <header className="su-modes-head">
          <h1 className="su-modes-title">Asset Manager</h1>
          <p className="su-modes-sub">What the forces are made of, and how they behave.</p>
        </header>
        <div className="su-modes-grid">
          {SECTIONS.map((s) => (
            <button
              key={s.to}
              className="su-mode"
              style={{ "--mode-accent": s.accent } as CSSProperties}
              onClick={() => navigate(s.to)}
            >
              <span className="su-mode-glyph" aria-hidden>
                {s.glyph}
              </span>
              <span className="su-mode-title">{s.title}</span>
              <span className="su-mode-blurb">{s.blurb}</span>
              <span className="su-mode-detail">{s.detail}</span>
              <span className="su-mode-cta">{s.cta} &rsaquo;</span>
            </button>
          ))}
        </div>
      </main>
    </div>
  );
}
