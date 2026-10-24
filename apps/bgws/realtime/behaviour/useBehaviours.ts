import { useSyncExternalStore } from "react";
import { getVersion, subscribe } from "./store";

/**
 * Re-render when behaviour templates or assignments change (in this tab).
 * Returns a version number: read the store's functions after calling it.
 */
export function useBehaviours(): number {
  return useSyncExternalStore(subscribe, getVersion, getVersion);
}
