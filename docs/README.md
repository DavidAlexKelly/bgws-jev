# Docs

| Folder | What it holds |
|---|---|
| `design/` | How the engine is built. `rust-engine.md` is the design the code cites by section (§); `multi-domain-plan.md`, `decision-flow-graphs.md`, `side-picture.md` and `interop-plan.md` are the plans it builds on. |
| `research/` | Our own analysis: `calibration-targets.md` (what each vignette is checked against, and why), `catalogue-csv-assessment.md` (the asset CSVs' quality), `modern-warfare-reference.md` (the sources behind the figures and the vignettes' bands, the `P…` and `R…` references). |
| `sources/` | The source material: raw PDFs (never edited), a Markdown note and CSV tables derived from each, leads not yet obtained, and an index. Its `README.md` sets the naming and layout rules. |
| `rules/` | `bgws-rulebook.pdf`, the original tabletop rules. |

All file names are lower case with hyphens. Sources are named
`<author-or-organisation>-<year>-<short-title>` (see `sources/README.md`).

The design documents were written while the engine was being ported from
an earlier TypeScript implementation; paths and some context in them
describe that time. `README.md` at the root and `protocol/PROTOCOL.md`
describe the project as it is.
