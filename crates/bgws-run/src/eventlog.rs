//! Event logs (§12, §15): JSONL by default, or a DuckDB database (built with
//! `--features duckdb`) when the file ends in `.duckdb`, so a game's events
//! can be queried with SQL. Either can be read back for the review.
//!
//! The DuckDB table is `events(t_us BIGINT, t DOUBLE, seq UBIGINT, kind
//! VARCHAR, side VARCHAR, entity UINTEGER, target UINTEGER, text VARCHAR,
//! data VARCHAR)`: `t` is seconds for queries, `t_us` the exact time, `data`
//! the event's figures as JSON text (`json_extract(data, '$.p')`).

use anyhow::{anyhow, Context, Result};
use bgws_core::LogEvent;
use std::io::{BufRead, Write};
use std::path::Path;

/// Somewhere events go as the game plays.
pub trait EventSink: Send {
    fn write(&mut self, events: &[LogEvent]) -> Result<()>;
}

struct Jsonl(std::io::BufWriter<std::fs::File>);

impl EventSink for Jsonl {
    fn write(&mut self, events: &[LogEvent]) -> Result<()> {
        for e in events {
            serde_json::to_writer(&mut self.0, e)?;
            self.0.write_all(b"\n")?;
        }
        self.0.flush()?;
        Ok(())
    }
}

/// Is this path a DuckDB database (by its extension)?
pub fn is_duckdb(path: &Path) -> bool {
    matches!(path.extension().and_then(|e| e.to_str()), Some("duckdb" | "ddb"))
}

/// A sink for `path`: DuckDB for `.duckdb`, JSONL otherwise.
pub fn open_sink(path: &Path) -> Result<Box<dyn EventSink>> {
    if is_duckdb(path) {
        return duck::open_sink(path);
    }
    Ok(Box::new(Jsonl(std::io::BufWriter::new(std::fs::File::create(path).with_context(|| format!("creating {}", path.display()))?))))
}

/// Read an event log back, in the order it was written.
pub fn read_log(path: &Path) -> Result<Vec<LogEvent>> {
    if is_duckdb(path) {
        return duck::read(path);
    }
    let f = std::io::BufReader::new(std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?);
    let mut out = Vec::new();
    for (i, line) in f.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        out.push(serde_json::from_str(&line).map_err(|e| anyhow!("line {}: {e}", i + 1))?);
    }
    Ok(out)
}

/// A query's result: column names and rows, as text.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn markdown(&self) -> String {
        let mut s = format!("| {} |\n|{}|\n", self.columns.join(" | "), self.columns.iter().map(|_| "---").collect::<Vec<_>>().join("|"));
        for r in &self.rows {
            s.push_str(&format!("| {} |\n", r.join(" | ")));
        }
        s
    }
}

/// Run SQL over an event log (a `.duckdb` file, or a JSONL log loaded into
/// memory first); the table is `events`.
pub fn query(path: &Path, sql: &str) -> Result<Table> {
    duck::query(path, sql)
}

/// The default summary: events by kind.
pub const SUMMARY_SQL: &str = "SELECT kind, count(*) AS n, round(min(t), 1) AS first_s, round(max(t), 1) AS last_s FROM events GROUP BY kind ORDER BY n DESC, kind";

#[cfg(feature = "duckdb")]
mod duck {
    use super::{EventSink, Table};
    use anyhow::{Context, Result};
    use bgws_core::{EntityId, LogEvent, SimTime};
    use duckdb::{params, Connection};
    use std::path::Path;

    const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS events(t_us BIGINT, t DOUBLE, seq UBIGINT, kind VARCHAR, side VARCHAR, entity UINTEGER, target UINTEGER, text VARCHAR, data VARCHAR)";

    struct Duck(Connection);

    impl EventSink for Duck {
        fn write(&mut self, events: &[LogEvent]) -> Result<()> {
            if events.is_empty() {
                return Ok(());
            }
            append(&self.0, events)
        }
    }

    fn append(conn: &Connection, events: &[LogEvent]) -> Result<()> {
        let mut app = conn.appender("events")?;
        for e in events {
            let data = if e.data.is_null() { None } else { Some(e.data.to_string()) };
            app.append_row(params![e.t.0, e.t.secs(), e.seq, e.kind, e.side.map(|s| s.as_str()), e.entity.map(|x| x.0), e.target.map(|x| x.0), e.text, data])?;
        }
        app.flush()?;
        Ok(())
    }

    pub fn open_sink(path: &Path) -> Result<Box<dyn EventSink>> {
        if path.exists() {
            std::fs::remove_file(path).with_context(|| format!("replacing {}", path.display()))?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Box::new(Duck(conn)))
    }

    pub fn read(path: &Path) -> Result<Vec<LogEvent>> {
        let conn = Connection::open(path)?;
        let mut stmt = conn.prepare("SELECT t_us, seq, kind, side, entity, target, text, data FROM events ORDER BY seq")?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, u64>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?, r.get::<_, Option<u32>>(4)?, r.get::<_, Option<u32>>(5)?, r.get::<_, String>(6)?, r.get::<_, Option<String>>(7)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (t, seq, kind, side, entity, target, text, data) = row?;
            out.push(LogEvent {
                t: SimTime(t),
                seq,
                kind,
                side: side.and_then(|s| s.parse().ok()),
                entity: entity.map(EntityId),
                target: target.map(EntityId),
                text,
                data: data.map(|d| serde_json::from_str(&d)).transpose()?.unwrap_or(serde_json::Value::Null),
            });
        }
        Ok(out)
    }

    pub fn query(path: &Path, sql: &str) -> Result<Table> {
        let conn = if super::is_duckdb(path) {
            Connection::open(path)?
        } else {
            let conn = Connection::open_in_memory()?;
            conn.execute_batch(SCHEMA)?;
            append(&conn, &super::read_log(path)?)?;
            conn
        };
        let mut stmt = conn.prepare(sql)?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let n = r.as_ref().column_count();
            out.push((0..n).map(|i| r.get::<_, duckdb::types::Value>(i).map(text)).collect::<Result<Vec<_>, _>>()?);
        }
        drop(rows);
        let columns = stmt.column_names();
        Ok(Table { columns, rows: out })
    }

    fn text(v: duckdb::types::Value) -> String {
        use duckdb::types::Value as V;
        match v {
            V::Null => String::new(),
            V::Boolean(b) => b.to_string(),
            V::TinyInt(x) => x.to_string(),
            V::SmallInt(x) => x.to_string(),
            V::Int(x) => x.to_string(),
            V::BigInt(x) => x.to_string(),
            V::HugeInt(x) => x.to_string(),
            V::UTinyInt(x) => x.to_string(),
            V::USmallInt(x) => x.to_string(),
            V::UInt(x) => x.to_string(),
            V::UBigInt(x) => x.to_string(),
            V::Float(x) => x.to_string(),
            V::Double(x) => x.to_string(),
            V::Text(s) => s,
            other => format!("{other:?}"),
        }
    }
}

#[cfg(not(feature = "duckdb"))]
mod duck {
    use super::{EventSink, Table};
    use anyhow::{bail, Result};
    use bgws_core::LogEvent;
    use std::path::Path;

    const NEEDS: &str = "DuckDB event logs need a build with `--features duckdb` (cargo build --release -p bgws-run --features duckdb)";

    pub fn open_sink(_: &Path) -> Result<Box<dyn EventSink>> {
        bail!(NEEDS)
    }
    pub fn read(_: &Path) -> Result<Vec<LogEvent>> {
        bail!(NEEDS)
    }
    pub fn query(_: &Path, _: &str) -> Result<Table> {
        bail!(NEEDS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bgws_core::{EntityId, Side, SimTime};

    fn events() -> Vec<LogEvent> {
        vec![
            LogEvent { t: SimTime::from_secs(1.5), seq: 0, kind: "sense.detect".into(), side: Some(Side::Blue), entity: Some(EntityId(3)), target: Some(EntityId(7)), text: "Blue spots Red".into(), data: serde_json::json!({ "p": 0.4 }) },
            LogEvent { t: SimTime::from_secs(9.0), seq: 1, kind: "fire.kill".into(), side: Some(Side::Red), entity: Some(EntityId(7)), target: None, text: "Red fires".into(), data: serde_json::Value::Null },
            LogEvent { t: SimTime::from_secs(9.0), seq: 2, kind: "fire.kill".into(), side: None, entity: None, target: None, text: "x".into(), data: serde_json::Value::Null },
        ]
    }

    #[test]
    fn jsonl_round_trips() {
        let dir = std::env::temp_dir().join(format!("bgws-eventlog-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("e.jsonl");
        let mut sink = open_sink(&p).unwrap();
        sink.write(&events()).unwrap();
        drop(sink);
        assert_eq!(read_log(&p).unwrap(), events());
    }

    #[cfg(feature = "duckdb")]
    #[test]
    fn duckdb_round_trips_and_answers_sql() {
        let dir = std::env::temp_dir().join(format!("bgws-eventlog-duck-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("e.duckdb");
        let mut sink = open_sink(&p).unwrap();
        sink.write(&events()[..1]).unwrap();
        sink.write(&events()[1..]).unwrap();
        drop(sink);
        assert_eq!(read_log(&p).unwrap(), events());
        let t = query(&p, SUMMARY_SQL).unwrap();
        assert_eq!(t.columns, vec!["kind", "n", "first_s", "last_s"]);
        assert_eq!(t.rows[0], vec!["fire.kill", "2", "9", "9"]);
        let p_det = query(&p, "SELECT CAST(json_extract(data, '$.p') AS DOUBLE) FROM events WHERE kind = 'sense.detect'").unwrap();
        assert_eq!(p_det.rows, vec![vec!["0.4".to_string()]]);
        // A JSONL log can be queried too.
        let j = dir.join("e.jsonl");
        let mut sink = open_sink(&j).unwrap();
        sink.write(&events()).unwrap();
        drop(sink);
        assert_eq!(query(&j, "SELECT count(*) FROM events").unwrap().rows, vec![vec!["3".to_string()]]);
    }
}
