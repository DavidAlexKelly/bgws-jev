/**
 * BgwsHome — the main menu.
 *
 * A title, the two things you can do, and behind them a real map drifting
 * slowly past: out of focus, dimmed, and inert, so it sets the scene without
 * competing with the menu.
 *
 * The background is the same global basemap Play uses (planetStore), so it
 * costs no extra Developer Console resources. If it cannot load, the page
 * simply shows the dark gradient underneath — the menu never depends on it.
 */

import { useCallback, useEffect, useState } from "react";
import type maplibregl from "maplibre-gl";
import { Link } from "react-router-dom";

import { DechoBasemap } from "@acc/decho-basemap/react";
import { planetStore } from "@/shared/map/dechoBasemapSetup";

// ── Background map ───────────────────────────────────────────────────────────

/** Where the drift starts: the southern Baltic, west of Gdańsk. */
const START = { lat: 54.45, lng: 14.2 };
const START_ZOOM = 7;
/** Screen pixels per second. Slow enough to notice only when you look for it. */
const DRIFT_PX_PER_SEC = 9;

function prefersReducedMotion(): boolean {
  return typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
}

/**
 * Pans the map eastward forever, one frame at a time. requestAnimationFrame
 * pauses itself in background tabs, and dt is clamped so returning to the tab
 * does not jump the map by however long you were away.
 */
function useDrift(map: maplibregl.Map | null) {
  useEffect(() => {
    if (!map || prefersReducedMotion()) return;
    let frame = 0;
    let last = performance.now();
    const step = (now: number) => {
      const dt = Math.min((now - last) / 1000, 0.1);
      last = now;
      map.panBy([DRIFT_PX_PER_SEC * dt, 0], { animate: false });
      frame = requestAnimationFrame(step);
    };
    frame = requestAnimationFrame(step);
    return () => cancelAnimationFrame(frame);
  }, [map]);
}

function BackgroundMap() {
  const [map, setMap] = useState<maplibregl.Map | null>(null);

  const onMapReady = useCallback((ready: maplibregl.Map) => {
    // Scenery, not a control: nothing about it should take input or focus.
    ready.dragPan.disable();
    ready.scrollZoom.disable();
    ready.boxZoom.disable();
    ready.dragRotate.disable();
    ready.doubleClickZoom.disable();
    ready.touchZoomRotate.disable();
    ready.keyboard.disable();
    ready.getCanvas().tabIndex = -1;
    setMap(ready);
  }, []);

  useDrift(map);

  return (
    <div style={bgLayer} aria-hidden="true">
      <DechoBasemap
        tiles={planetStore}
        spawnLat={START.lat}
        spawnLong={START.lng}
        spawnZoom={START_ZOOM}
        navigationControl={false}
        scaleControl={false}
        onMapReady={onMapReady}
        renderLoading={() => null}
        renderError={() => null}
        style={{ position: "absolute", inset: 0 }}
      />
    </div>
  );
}

// ── Menu ─────────────────────────────────────────────────────────────────────

interface MenuItem {
  to: string;
  title: string;
  blurb: string;
  glyph: string;
}

const MENU: MenuItem[] = [
  {
    to: "/bgws/play",
    title: "Play",
    blurb: "Fight an engagement — turn-based or real-time, with fog of war",
    glyph: "\u25b6",
  },
  {
    to: "/bgws/assets",
    title: "Asset Explorer",
    blurb: "Browse the equipment catalogue — stats, armament, photographs",
    glyph: "\u26ed",
  },
];

export default function BgwsHome() {
  return (
    <div style={page}>
      <style>{MENU_CSS}</style>
      <BackgroundMap />
      <div style={vignette} aria-hidden="true" />

      <main style={content}>
        <header style={{ textAlign: "center" }}>
          <div style={kicker}>Battlegroup Wargame System</div>
          <h1 style={title}>APBG Wargaming Hub</h1>
          <div style={rule} aria-hidden="true" />
        </header>

        <nav aria-label="Main menu" style={menu}>
          {MENU.map((item) => (
            <Link key={item.to} to={item.to} className="apbg-menu-item">
              <span className="apbg-menu-glyph" aria-hidden="true">
                {item.glyph}
              </span>
              <span style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                <span className="apbg-menu-title">{item.title}</span>
                <span className="apbg-menu-blurb">{item.blurb}</span>
              </span>
              <span className="apbg-menu-chevron" aria-hidden="true">
                &rsaquo;
              </span>
            </Link>
          ))}
        </nav>
      </main>
    </div>
  );
}

// ── Styles ───────────────────────────────────────────────────────────────────

const ACCENT = "#7fb27f";

const page: React.CSSProperties = {
  position: "relative",
  height: "100vh",
  overflow: "hidden",
  // Shows through if the map is still loading or failed to.
  background: "radial-gradient(ellipse at 50% 40%, #13202f 0%, #070b12 70%)",
  color: "#e9ecfb",
};

const bgLayer: React.CSSProperties = {
  position: "absolute",
  // Overscan, so the blur does not pull a soft dark edge in from outside.
  inset: "-40px",
  filter: "blur(3px) saturate(0.7) brightness(0.75)",
  pointerEvents: "none",
};

const vignette: React.CSSProperties = {
  position: "absolute",
  inset: 0,
  pointerEvents: "none",
  background:
    "radial-gradient(ellipse at 50% 45%, rgba(7,11,18,0.25) 0%, rgba(7,11,18,0.7) 60%, rgba(7,11,18,0.95) 100%)",
};

const content: React.CSSProperties = {
  position: "relative",
  height: "100%",
  display: "flex",
  flexDirection: "column",
  alignItems: "center",
  justifyContent: "center",
  gap: 44,
  padding: 24,
};

const kicker: React.CSSProperties = {
  font: "600 11px/1 var(--font-mono, monospace)",
  letterSpacing: "0.32em",
  textTransform: "uppercase",
  color: ACCENT,
  marginBottom: 14,
};

const title: React.CSSProperties = {
  margin: 0,
  fontSize: "clamp(34px, 5.4vw, 64px)",
  fontWeight: 800,
  letterSpacing: "0.06em",
  textTransform: "uppercase",
  lineHeight: 1.05,
  textShadow: "0 2px 24px rgba(0,0,0,0.6)",
};

const rule: React.CSSProperties = {
  width: 120,
  height: 2,
  margin: "22px auto 0",
  background: `linear-gradient(90deg, transparent, ${ACCENT}, transparent)`,
};

const menu: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  gap: 14,
  width: "min(480px, 100%)",
};

// Hover and focus states need real CSS; inline styles cannot express them.
const MENU_CSS = `
.apbg-menu-item {
  display: flex;
  align-items: center;
  gap: 18px;
  padding: 18px 22px;
  border: 1px solid rgba(233, 236, 251, 0.12);
  border-left: 3px solid rgba(127, 178, 127, 0.45);
  border-radius: 6px;
  background: rgba(10, 16, 26, 0.62);
  backdrop-filter: blur(6px);
  color: #e9ecfb;
  text-decoration: none;
  transition: transform 160ms ease, background 160ms ease, border-color 160ms ease, box-shadow 160ms ease;
}
.apbg-menu-item:hover,
.apbg-menu-item:focus-visible {
  transform: translateX(4px);
  background: rgba(18, 30, 44, 0.82);
  border-color: rgba(127, 178, 127, 0.55);
  border-left-color: ${ACCENT};
  box-shadow: 0 6px 28px rgba(0, 0, 0, 0.45), 0 0 0 1px rgba(127, 178, 127, 0.15);
  outline: none;
}
.apbg-menu-glyph {
  width: 36px;
  font-size: 24px;
  text-align: center;
  color: ${ACCENT};
}
.apbg-menu-title {
  font-size: 19px;
  font-weight: 700;
  letter-spacing: 0.08em;
  text-transform: uppercase;
}
.apbg-menu-blurb {
  font: 12px/1.4 var(--font-mono, monospace);
  color: #8a93b4;
}
.apbg-menu-chevron {
  margin-left: auto;
  font-size: 26px;
  color: rgba(233, 236, 251, 0.35);
  transition: color 160ms ease, transform 160ms ease;
}
.apbg-menu-item:hover .apbg-menu-chevron,
.apbg-menu-item:focus-visible .apbg-menu-chevron {
  color: ${ACCENT};
  transform: translateX(3px);
}
@media (prefers-reduced-motion: reduce) {
  .apbg-menu-item, .apbg-menu-chevron { transition: none; }
  .apbg-menu-item:hover, .apbg-menu-item:focus-visible { transform: none; }
}
`;
