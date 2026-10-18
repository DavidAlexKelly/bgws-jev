import {
  createClient,
  createPlatformClient,
  type Client,
  type PlatformClient,
} from "@osdk/client";
import { createPublicOauthClient, type PublicOauthClient } from "@osdk/oauth";
import { $ontologyRid } from "@dk-real-time-ai-wargaming/sdk";

function getMetaTagContent(tagName: string): string {
  const elements = document.querySelectorAll(`meta[name="${tagName}"]`);
  const element = elements.item(elements.length - 1);
  const value = element ? element.getAttribute("content") : null;
  if (value == null || value === "") {
    throw new Error(`Meta tag ${tagName} not found or empty`);
  }
  if (value.match(/%.+%/)) {
    throw new Error(
      `Meta tag ${tagName} contains placeholder value. Please add ${value.replace(
        /%/g,
        ""
      )} to your .env files`
    );
  }
  return value;
}

export const foundryUrl = getMetaTagContent("osdk-foundryUrl");
const clientId = getMetaTagContent("osdk-clientId");
const redirectUrl = getMetaTagContent("osdk-redirectUrl");

/**
 * Everything BGWS touches. Each of these must ALSO be enabled on the
 * Developer Console application's "OAuth & scopes" page, otherwise the
 * corresponding calls fail with a 403:
 *
 *   ontologies   — the bgwsCommander* AIP functions (AI commander)
 *   datasets     — terrain rasters, basemap tiles/assets (decho-basemap/-elevation)
 *   mediasets    — unit images, globe basemap
 *   sql-queries  — equipment profiles, asset cards, curated assets
 *   streams      — the game event stream
 */
const scopes = [
  "api:use-ontologies-read",
  "api:use-ontologies-write",
  "api:use-datasets-read",
  "api:use-mediasets-read",
  "api:use-mediasets-write",
  "api:mediasets-read",
  "api:third-party-applications",
  "api:sql-queries-execute",
  "api:sql-queries-read",
  "api:streams-read",
  "api:streams-write",
];

export const auth: PublicOauthClient = createPublicOauthClient(
  clientId,
  foundryUrl,
  redirectUrl,
  { scopes }
);

/** Ontology SDK client — functions, objects, actions. */
export const client: Client = createClient(foundryUrl, $ontologyRid, auth);

/** Platform SDK client — datasets, media sets, SQL, streams. */
export const platformClient: PlatformClient = createPlatformClient(foundryUrl, auth);

export default client;
