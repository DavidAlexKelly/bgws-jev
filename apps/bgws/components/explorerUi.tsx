/**
 * Building blocks shared by the Asset Explorer's catalogues.
 *
 * Presentation only — every piece takes plain values and callbacks, so the
 * data rules (what counts as empty, which note qualifies which figure) stay
 * in the explorers and their tested data modules. Styling is in
 * ../explorer.css; see the responsive model described there.
 */

import { useEffect, type ReactNode } from "react";

import type { ResolvedImage } from "../data/unitImages";

// ─── Layout ────────────────────────────────────────────────────────────────

/**
 * The three panes. `hasSelection` drives the phone layout (list OR detail);
 * `filtersOpen` drives the drawer below 1280px. Escape closes the drawer.
 */
export function ExplorerLayout({
  filtersOpen,
  hasSelection,
  onCloseFilters,
  filters,
  list,
  detail,
}: {
  filtersOpen: boolean;
  hasSelection: boolean;
  onCloseFilters: () => void;
  filters: ReactNode;
  list: ReactNode;
  detail: ReactNode;
}) {
  useEffect(() => {
    if (!filtersOpen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCloseFilters();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [filtersOpen, onCloseFilters]);

  return (
    <div
      className="ax-layout"
      data-filters-open={filtersOpen}
      data-has-selection={hasSelection}
    >
      {filters}
      <button
        type="button"
        className="ax-scrim"
        aria-label="Close filters"
        tabIndex={-1}
        onClick={onCloseFilters}
      />
      {list}
      {detail}
    </div>
  );
}

export function FiltersPanel({
  activeCount,
  onClear,
  onClose,
  children,
}: {
  activeCount: number;
  onClear: () => void;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <aside className="ax-filters" aria-label="Filters">
      <div className="ax-filters-head">
        <h2>Filters</h2>
        {activeCount > 0 && (
          <button type="button" className="ax-btn ax-btn-sm ax-btn-ghost ax-btn-danger" onClick={onClear}>
            Clear {activeCount}
          </button>
        )}
        <button
          type="button"
          className="ax-btn ax-btn-sm ax-btn-ghost ax-drawer-close"
          onClick={onClose}
          aria-label="Close filters"
        >
          ✕
        </button>
      </div>
      {children}
    </aside>
  );
}

/** Only rendered below 1280px, where the filters are a drawer. */
export function FiltersButton({
  activeCount,
  onClick,
}: {
  activeCount: number;
  onClick: () => void;
}) {
  return (
    <button type="button" className="ax-btn ax-only-drawer" onClick={onClick}>
      ☰ Filters
      {activeCount > 0 && <span className="ax-count">{activeCount}</span>}
    </button>
  );
}

export function FilterSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <div className="ax-facet-label" style={{ marginBottom: 8 }}>
        {title}
      </div>
      {children}
    </section>
  );
}

export function SearchInput({
  value,
  onChange,
  placeholder,
  label,
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  label: string;
}) {
  return (
    <div className="ax-search">
      <input
        type="search"
        className="ax-input"
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        aria-label={label}
      />
    </div>
  );
}

export function DetailFrame({ onBack, children }: { onBack: () => void; children: ReactNode }) {
  return (
    <div className="ax-detail-inner">
      <button
        type="button"
        className="ax-btn ax-btn-ghost ax-detail-back"
        onClick={onBack}
        style={{ marginLeft: -8, marginBottom: 8 }}
      >
        ‹ Back to list
      </button>
      {children}
    </div>
  );
}

export function DetailPlaceholder({ children }: { children: ReactNode }) {
  return (
    <div className="ax-empty" style={{ paddingTop: "18vh" }}>
      <div style={{ fontSize: 28, marginBottom: 8, opacity: 0.5 }} aria-hidden="true">
        ◫
      </div>
      {children}
    </div>
  );
}

export function ListSkeleton({ rows = 8 }: { rows?: number }) {
  return (
    <div aria-busy="true" aria-label="Loading">
      {Array.from({ length: rows }, (_, index) => (
        <div key={index} className="ax-skeleton" />
      ))}
    </div>
  );
}

// ─── Detail content ────────────────────────────────────────────────────────

export function Group({
  title,
  wide = false,
  children,
}: {
  title: string;
  wide?: boolean;
  children: ReactNode;
}) {
  return (
    <section className={wide ? "ax-group ax-group-wide" : "ax-group"}>
      <h3>{title}</h3>
      {children}
    </section>
  );
}

/** A labelled figure. "—" renders muted, so absences read as absences. */
export function Row({ label, value, note }: { label: string; value: string; note?: string }) {
  return (
    <div className="ax-row">
      <span className="ax-row-label">{label}</span>
      <span className="ax-row-value" data-empty={value === "—"}>
        {value}
        {note && <span className="ax-row-note">{note}</span>}
      </span>
    </div>
  );
}

export function Kpis({ items }: { items: { label: string; value: string }[] }) {
  return (
    <div className="ax-kpis">
      {items.map((item) => (
        <div key={item.label} className="ax-kpi">
          <div className="ax-kpi-value" data-empty={item.value === "—"}>
            {item.value}
          </div>
          <div className="ax-kpi-label">{item.label}</div>
        </div>
      ))}
    </div>
  );
}

/**
 * The photograph, or a sentence saying why there is none. The three empty
 * states are different facts — still loading, nothing in the set, lookup
 * failed — and are kept distinct.
 */
export function ImageFrame({
  image,
  alt,
  missing,
  failureHint,
  caption,
}: {
  image: ResolvedImage | null;
  alt: string;
  missing: string;
  failureHint: string;
  caption?: ReactNode;
}) {
  return (
    <div>
      <div className="ax-image">
        {image === null && <div className="ax-skeleton" style={{ position: "absolute", inset: 0, margin: 0, height: "auto" }} />}
        {image?.url && <img src={image.url} alt={alt} />}
        {image !== null && !image.url && !image.error && (
          <div className="ax-image-note">{missing}</div>
        )}
        {image?.error && (
          <div className="ax-image-note" data-tone="bad">
            Image lookup failed — {image.error}
            <br />
            {failureHint}
          </div>
        )}
      </div>
      {caption && <div className="ax-image-caption">{caption}</div>}
    </div>
  );
}
