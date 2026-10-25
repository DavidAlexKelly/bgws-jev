// ── bgws/data/templateStore.ts ─────────────────────────────────────────────
// Behaviour templates in Foundry (DECISION_FLOW_GRAPHS.md §6, step 9): the
// `BgwsBehaviourTemplate` object type, read through the Ontology REST API v2
// and written through its create/edit/delete actions, with the app's own
// credentials — as the event stream does (data/eventStream.ts).
//
// NEEDS, in Foundry: the object type and the three actions below in the
// app's ontology, and the ontology write scope (api:use-ontologies-write),
// already requested in client.ts.

import { $ontologyRid } from "@dk-real-time-ai-wargaming/sdk";
import type { SharedTemplate, TemplateRemote } from "../realtime/behaviour/shared";

export const TEMPLATE_OBJECT_TYPE = "BgwsBehaviourTemplate";
export const TEMPLATE_ACTIONS = { create: "create-bgws-behaviour-template", edit: "edit-bgws-behaviour-template", remove: "delete-bgws-behaviour-template" } as const;

type Fetch = (url: string, init: { method: string; headers: Record<string, string>; body?: string }) => Promise<{ ok: boolean; status: number; json(): Promise<unknown>; text(): Promise<string> }>;

/**
 * A remote over Foundry's Ontology API. `ctx` is the app's platform client
 * (where Foundry is, and a token for it); `fetchImpl` defaults to the
 * browser's fetch. Tests pass their own.
 */
export function foundryTemplateRemote(
  ctx: { baseUrl: string; tokenProvider: () => Promise<string> },
  ontology: string = $ontologyRid,
  fetchImpl: Fetch = fetch as unknown as Fetch,
): TemplateRemote {
  const base = () => `${ctx.baseUrl.replace(/\/+$/, "")}/api/v2/ontologies/${encodeURIComponent(ontology)}`;
  const call = async (method: string, path: string, body?: unknown) => {
    const response = await fetchImpl(`${base()}${path}`, {
      method,
      headers: { Authorization: `Bearer ${await ctx.tokenProvider()}`, "Content-Type": "application/json" },
      ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
    });
    if (!response.ok) throw new Error(`templates ${response.status}: ${(await response.text().catch(() => "")).slice(0, 300)}`);
    return response.json();
  };
  const apply = (action: string, parameters: Record<string, unknown>) => call("POST", `/actions/${action}/apply`, { parameters });
  return {
    async list() {
      const out: SharedTemplate[] = [];
      let pageToken: string | undefined;
      do {
        const page = (await call("GET", `/objects/${TEMPLATE_OBJECT_TYPE}?pageSize=500${pageToken ? `&pageToken=${encodeURIComponent(pageToken)}` : ""}`)) as { data: SharedTemplate[]; nextPageToken?: string };
        out.push(...page.data);
        pageToken = page.nextPageToken;
      } while (pageToken);
      return out;
    },
    create: async (t) => void (await apply(TEMPLATE_ACTIONS.create, { ...t })),
    edit: async (t) => void (await apply(TEMPLATE_ACTIONS.edit, { ...t })),
    remove: async (id) => void (await apply(TEMPLATE_ACTIONS.remove, { templateId: id })),
  };
}
