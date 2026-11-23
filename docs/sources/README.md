# Sources

Everything the model's figures come from. The PDFs are the **raw** record
and are never edited. What the project uses is the **derived** form next to
them: a Markdown note per source and CSV tables of the numbers taken from
it. Code and docs cite the note or the table, not the PDF.

## Layout

| Folder | What goes in it |
|---|---|
| `raw/<topic>/` | The original files, renamed only. One topic folder per subject (below). |
| `notes/` | One Markdown note per raw file, with the **same stem**: what the source is, what in it we use, the figures with page numbers, and its limits. Short datasheets from one maker may share a note named for the family (`rheinmetall-datasheets-medium-calibre.md`), which lists the files it covers. |
| `data/` | CSV tables extracted from the sources, one per table, named `<note-stem>--<table>.csv`. |
| `leads/` | Sources we know of but don't have yet: what each is said to contain and where to get it. |
| `index.csv` | Every source: id, file, title, author or organisation, date, origin, topic, and how far it has been read. |
| `rheinmetall-crawl.tsv` | The record of where each Rheinmetall sheet was downloaded from. |

Topic folders: `sensing` (detection, optics, sensors), `missiles`,
`ballistics` (penetration and armour mechanics), `ammunition` (datasheets
for rounds), `weapons` (guns, mounts and weapon systems), `equipment`
(platform reference guides), `test-reports` (trials and test results), `doctrine` (field manuals and techniques publications), `simulation` (other simulations, data links and interoperability, combat-modelling commentary),
`other` (kept but off-topic).

## File names

`<author-or-organisation>-<year>-<short-title>.<ext>`, all lower case,
words joined by hyphens, no spaces.

- **Author:** the first author's surname, or the organisation for a
  corporate document (`rheinmetall`, `tradoc`, `dote`). A handbook series
  gets a short series name (`irhb` for the *Infrared & Electro-Optical
  Systems Handbook*).
- **Year:** of publication. `nd` when it is not known.
- **Title:** a few words, enough to tell it from its neighbours. Keep
  model and calibre designations (`apfsds-t-30x173-pmc287`). Volume numbers
  go before the title (`irhb-1993-v4-...`).
- The original file name, report number (DTIC `ADA…`), document code
  (Rheinmetall `D129e0721`) and URL are kept in `index.csv`, not in the
  name.

Docs elsewhere in the repository use the same style: lower case and
hyphens (`design/rust-engine.md`, `research/calibration-targets.md`).

## Status in `index.csv`

| Status | Meaning |
|---|---|
| `raw` | Filed and named; not yet read. |
| `noted` | A note exists; the figures we need are written down with page numbers. |
| `extracted` | Its tables are in `data/` as well. |
| `applied` | Figures from it are used in the model or the catalogue (the note says where). |
| `off-topic` | Kept for the record; not a source for the model. |

## Adding a source

1. Put the file in `raw/<topic>/` under the naming rule.
2. Add its row to `index.csv` with status `raw`.
3. Read it, write `notes/<stem>.md`, and move the status on.
