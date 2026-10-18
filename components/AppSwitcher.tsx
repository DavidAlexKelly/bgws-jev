/**
 * AppSwitcher —floating pill button that opens a small popover menu for
 * jumping between the BGWS sections.
 *
 * In the Mapplications repo this switched between the sibling apps (ORBAT
 * Manager, Scenario Planner, …). BGWS now lives in its own repo with nothing
 * to switch to, so the same control — same name, same look, same `.asw-*`
 * styles, so none of the BGWS screens needed to change — now lists the BGWS
 * sections instead.
 */

import { useEffect, useRef, useState } from "react";
import { useLocation, useNavigate } from "react-router-dom";

// ── Section catalogue ─────────────────────────────────────────────────────────

const SECTIONS = [
  { path: "/bgws", label: "Main menu", icon: "▦", sub: "APBG Wargaming Hub", accent: "#e8c547" },
  { path: "/bgws/play", label: "Play", icon: "▶", sub: "Turn-based and real-time games", accent: "#81c784" },
  { path: "/bgws/assets", label: "Asset explorer", icon: "☰", sub: "Equipment, profiles and cards", accent: "#ffb74d" },
] as const;

function isActivePath(sectionPath: string, pathname: string): boolean {
  const here = pathname.replace(/\/+$/, "") || "/";
  if (sectionPath === "/bgws") return here === "/bgws" || here === "/";
  return here === sectionPath || here.startsWith(`${sectionPath}/`);
}

// ── Styles (injected once as a stylesheet node) ───────────────────────────────

const STYLE = `
.asw-btn {
  display: flex; align-items: center; gap: 6px;
  background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.1);
  border-radius: 3px; cursor: pointer; padding: 0 10px; height: 28px;
  font-size: 10px; font-weight: 700; letter-spacing: 0.1em; text-transform: uppercase;
  color: rgba(255,255,255,0.45); font-family: inherit;
  transition: background 0.13s, border-color 0.13s, color 0.13s;
  flex-shrink: 0;
}
.asw-btn:hover { background: rgba(255,255,255,0.08); border-color: rgba(255,255,255,0.22); color: rgba(255,255,255,0.75); }
.asw-btn__grid { font-size: 13px; line-height: 1; opacity: 0.7; }
.asw-btn__chevron { font-size: 8px; opacity: 0.5; }

.asw-popover {
  position: fixed; z-index: 9999;
  background: #0f1117; border: 1px solid #2e3449;
  border-radius: 6px; padding: 6px;
  box-shadow: 0 12px 36px rgba(0,0,0,0.65);
  min-width: 210px;
  animation: aswFadeIn 0.12s ease;
}
@keyframes aswFadeIn { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: none; } }

.asw-item {
  display: flex; align-items: center; gap: 10px;
  padding: 8px 10px; border-radius: 4px; cursor: pointer;
  background: none; border: none; font-family: inherit; width: 100%; text-align: left;
  transition: background 0.12s;
  text-decoration: none;
}
.asw-item:hover { background: rgba(255,255,255,0.06); }
.asw-item--active { background: rgba(255,255,255,0.04); cursor: default; }

.asw-item__icon {
  width: 28px; height: 28px; border-radius: 4px;
  display: flex; align-items: center; justify-content: center;
  font-size: 14px; flex-shrink: 0;
  background: rgba(255,255,255,0.06);
}
.asw-item__text { flex: 1; min-width: 0; }
.asw-item__label { font-size: 12px; font-weight: 600; color: #e9ecfb; letter-spacing: 0.03em; }
.asw-item--active .asw-item__label { color: rgba(255,255,255,0.45); }
.asw-item__sub { font-size: 10px; color: #6a7292; margin-top: 1px; }
.asw-item__active-dot { width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0; }
`.trim();

let styleInjected = false;
function injectStyle() {
  if (styleInjected) return;
  styleInjected = true;
  const el = document.createElement("style");
  el.textContent = STYLE;
  document.head.appendChild(el);
}

// ── Component ─────────────────────────────────────────────────────────────────

export function AppSwitcher() {
  injectStyle();

  const navigate = useNavigate();
  const location = useLocation();
  const [open, setOpen] = useState(false);
  const [popoverStyle, setPopoverStyle] = useState<React.CSSProperties>({});
  const btnRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);

  // Most specific match wins, so /bgws/play/realtime shows "Play".
  const current = [...SECTIONS].reverse().find((s) => isActivePath(s.path, location.pathname));

  function handleOpen() {
    if (!btnRef.current) {
      setOpen((v) => !v);
      return;
    }
    const rect = btnRef.current.getBoundingClientRect();
    setPopoverStyle({ top: rect.bottom + 6, right: window.innerWidth - rect.right });
    setOpen((v) => !v);
  }

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    function onDown(e: MouseEvent) {
      if (
        popoverRef.current &&
        !popoverRef.current.contains(e.target as Node) &&
        btnRef.current &&
        !btnRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") setOpen(false);
    }
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  return (
    <>
      <button
        ref={btnRef}
        className="asw-btn"
        onClick={handleOpen}
        aria-haspopup="true"
        aria-expanded={open}
        title="Switch section"
      >
        <span className="asw-btn__grid">⊞</span>
        <span>{current?.label ?? "BGWS"}</span>
        <span className="asw-btn__chevron">{open ? "▲" : "▼"}</span>
      </button>

      {open && (
        <div
          ref={popoverRef}
          className="asw-popover"
          style={popoverStyle}
          role="menu"
          aria-label="Switch section"
        >
          {SECTIONS.map((section, i) => {
            const isActive = current?.path === section.path;
            return (
              <button
                key={section.path}
                role="menuitem"
                className={`asw-item${isActive ? " asw-item--active" : ""}`}
                style={{ marginTop: i === 0 ? 0 : 2 }}
                onClick={() => {
                  if (!isActive) navigate(section.path);
                  setOpen(false);
                }}
                aria-current={isActive ? "page" : undefined}
              >
                <span
                  className="asw-item__icon"
                  style={{ color: section.accent, background: `${section.accent}18` }}
                >
                  {section.icon}
                </span>
                <span className="asw-item__text">
                  <div className="asw-item__label">{section.label}</div>
                  <div className="asw-item__sub">{section.sub}</div>
                </span>
                {isActive && (
                  <span
                    className="asw-item__active-dot"
                    style={{ background: section.accent }}
                    aria-hidden="true"
                  />
                )}
              </button>
            );
          })}
        </div>
      )}
    </>
  );
}

export default AppSwitcher;
