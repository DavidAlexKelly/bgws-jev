# BGWS — Battlegroup Wargame System

A Foundry-hosted React app for playing battlegroup-level engagements on real
terrain, turn-based or real-time, with optional AI commanders. Moved here from
the Mapplications repo.

## Screens

| Route          | Screen          | Source                                    |
| -------------- | --------------- | ----------------------------------------- |
| `/bgws`        | Home            | `src/apps/bgws/Home.tsx`                  |
| `/bgws/play`   | Play            | `src/apps/bgws/Play.tsx` → `TurnPlay.tsx` / `realtime/RealtimePlay.tsx` |
| `/bgws/assets` | Asset explorer  | `src/apps/bgws/AssetExplorer.tsx`         |

`/` and any unknown path redirect to `/bgws`. Design notes and the rulebook are
in `src/apps/bgws/docs/`.

## Layout

- `src/apps/bgws/` — the app: `rules/` (turn-based rules engine), `realtime/`
  (real-time engine and overlays), `lib/` (terrain, movement, line of sight,
  fog of war), `data/` (Foundry data access).
- `src/shared/` — map setup (`@acc/decho-basemap`), terrain rasters, SQL and
  CSV helpers, MGRS overlay.
- `src/client.ts` — OAuth client, Ontology client (`client`) and Platform
  client (`platformClient`).

## Foundry dependencies

- **Ontology SDK** `@dk-real-time-ai-wargaming/sdk`: the `bgwsCommander*` AIP
  functions behind the AI commander.
- **`@acc/*` packages** come from a separate artifacts repository, added to
  this repo as a backing repository.
- **Developer Console**: the OAuth scopes listed in `src/client.ts`, and the
  datasets and media sets BGWS reads (terrain, asset cards, profiles, unit
  images, event stream, basemap), must be enabled on the application. Missing
  resources show up as 403s and blank maps or panels.
- **Optional**: `VITE_OPENROUTER_API_KEY` enables the OpenRouter-backed JEV
  commander. Vite inlines `VITE_*` values into the shipped bundle, so the key is
  visible to every user of the site.

## Developing

A `FOUNDRY_TOKEN` environment variable is required to authenticate with the
NPM registry. When developing locally you may use the token used to git clone
the repository (may only be valid for 7 days), or generate a longer-lived
token [inside Foundry](https://www.palantir.com/docs/foundry/platform-security-third-party/user-generated-tokens/#generation).

```sh
npm install
npm run dev          # local, http://localhost:8080
npm run dev:remote   # in a Code Workspace; open the preview panel
```

Before committing: `npm run lint && npm run test && npm run build`. CI runs the
same three and fails on any lint warning.

Development configuration is in `.env.development` / `.env.code-workspaces`.
For local development, CORS must allow `http://localhost:8080` and the OAuth
client must allow `http://localhost:8080/auth/callback` as a redirect URL.

## Deploying

Foundry CI deploys a production build to Foundry website hosting whenever a
git tag is pushed:

```sh
git tag <x.y.z>
git push origin tag <x.y.z>
```

Production configuration is in `.env.production`. On tag builds CI also runs
`src/env.test.ts`, which fails if `VITE_FOUNDRY_REDIRECT_URL` still contains
the template placeholder — set it to `https://<your-subdomain>/auth/callback`
once a website-hosting subdomain is registered, and allow that URL on the OAuth
client. Set `site.uploadOnly` in `foundry.config.json` to `true` to upload
versions without deploying them immediately.
