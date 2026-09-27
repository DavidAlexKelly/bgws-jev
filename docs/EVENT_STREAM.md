# The event-log stream

The real-time mode sends every line of its feed to a Foundry stream as it happens:

- **Stream:** `ri.foundry.main.dataset.032c716d-b571-4ca4-a683-e4e555757d24`, branch `master`.
- **Rows:** one per feed line. Each row has the shared columns, plus one struct matching `entryType` (`event`, `shot`, `decision` or `flag`). The other three structs are null.
- **Where the code is:**
  - `realtime/engine/feed.ts`, `streamRow`: turns a feed line into a row.
  - `data/eventStreamQueue.ts`: batches the rows.
  - `data/eventStream.ts`: sends each batch to Foundry.

## How it is sent

The app sends a batch every 1.5 s, of up to 200 rows, to:

```
POST /api/v2/highScale/streams/datasets/{rid}/streams/master/publishRecords?preview=true
body: {"records": [...]}
```

It uses the app's own Foundry credentials.

- **Timestamps** (`emittedAt`) are sent as epoch milliseconds.
- **If a batch fails**, it is retried twice and then dropped. The reason is shown on screen, and the game is never held up.
- **Each Play or "Run scenario again"** gets a new `runId`.

**Needs, in Developer Console:**
- the stream's dataset as a permitted resource;
- the streams write scope on the app's client.

It can be turned off in setup, under "Stream the event log to Foundry".

## Columns

| Column | Type | Null | What |
|---|---|---|---|
| runId | STRING | no | One run of a scenario |
| sequence | LONG | no | Order within the run |
| emittedAt | TIMESTAMP | no | Wall-clock time |
| simTimeS | INTEGER | no | Game time, seconds |
| gameSeed | STRING | no | The scenario's seed |
| entryType | STRING | no | event · shot · decision · flag |
| side | STRING | no | Side of the unit involved (the firer, for a shot) |
| unitId | STRING | yes | The unit concerned; null for side-wide flags |
| unitLat, unitLng | DOUBLE | yes | Where that unit was |
| text | STRING | no | The feed line, as on screen |
| event | STRUCT | yes | kind, detail, severe, info, aboutId, located, bearingDeg |
| shot | STRUCT | yes | firerId, targetId, targetSide, result, narrative, rangeM, rounds, hits, knockedOut, pHit |
| decision | STRUCT | yes | decisionPoint, question, optionId, summary, chosenBy, fallback, confidence, latencyMs, askedAtS, rationale, options[] {id, summary, probability} |
| flag | STRUCT | yes | kind, text |
