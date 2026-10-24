/**
 * The Behaviour section of an asset's page: the behaviour template it uses,
 * swappable for any other.
 */

import { Link } from "react-router-dom";
import {
  allTemplates,
  assignTemplate,
  defaultTemplateFor,
  isAssigned,
  templateFor,
} from "../realtime/behaviour/store";
import { useBehaviours } from "../realtime/behaviour/useBehaviours";
import "../behaviour.css";

export function BehaviourAssignment({
  assetId,
  assetName,
  targetClass,
}: {
  assetId: string;
  assetName: string;
  targetClass?: string | null;
}) {
  useBehaviours();
  const templates = allTemplates();
  const current = templateFor(assetId, targetClass);
  const fallback = defaultTemplateFor(targetClass);
  const nameOf = (id: string) => templates.find((t) => t.id === id)?.name ?? id;

  return (
    <div className="bm-assign">
      <div className="bm-assign-row">
        <select
          className="bm-select"
          aria-label={`Behaviour for ${assetName}`}
          value={current}
          onChange={(e) => assignTemplate(assetId, e.target.value, targetClass)}
        >
          {templates.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name ?? t.id}
              {t.id === fallback ? " (default)" : ""}
            </option>
          ))}
        </select>
        {isAssigned(assetId, targetClass) && (
          <button className="bm-btn" onClick={() => assignTemplate(assetId, null, targetClass)}>
            Reset to default
          </button>
        )}
      </div>
      <div className="bm-item-meta">
        {isAssigned(assetId, targetClass)
          ? `Swapped from its default, ${nameOf(fallback)}.`
          : `Using its default behaviour, ${nameOf(fallback)}.`}{" "}
        <Link className="bm-link" to={`/bgws/assets/behaviours?id=${encodeURIComponent(current)}`}>
          Open in Behaviour Manager ›
        </Link>
      </div>
    </div>
  );
}
