//! Export of every stored session as CSV or JSON.
//!
//! One record per session: local start/end (ISO 8601 with offset), length in
//! seconds, app name and executable. Hidden shell processes are included:
//! an export is the raw record, not a report.

use std::io::{self, Write};

use chrono::{Local, TimeZone};

use crate::report::Catalog;
use crate::store::{Result, Store};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub start: i64,
    pub end: i64,
    pub app: String,
    pub exe_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
        }
    }
}

pub fn records(store: &Store) -> Result<Vec<Record>> {
    let cat = Catalog::load(store)?;
    Ok(store
        .all_spans()?
        .into_iter()
        .map(|s| {
            let (app, exe) = cat.describe(s.app_id).unwrap_or(("?", ""));
            Record { start: s.start, end: s.end, app: app.to_string(), exe_path: exe.to_string() }
        })
        .collect())
}

fn iso(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .earliest()
        .map(|d| d.format("%Y-%m-%dT%H:%M:%S%:z").to_string())
        .unwrap_or_default()
}

pub fn write(w: &mut impl Write, format: Format, records: &[Record]) -> io::Result<()> {
    match format {
        Format::Csv => write_csv(w, records, iso),
        Format::Json => write_json(w, records, iso),
    }
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn write_csv(w: &mut impl Write, records: &[Record], time: fn(i64) -> String) -> io::Result<()> {
    writeln!(w, "start,end,seconds,app,exe_path")?;
    for r in records {
        writeln!(
            w,
            "{},{},{},{},{}",
            time(r.start),
            time(r.end),
            r.end - r.start,
            csv_field(&r.app),
            csv_field(&r.exe_path)
        )?;
    }
    Ok(())
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn write_json(w: &mut impl Write, records: &[Record], time: fn(i64) -> String) -> io::Result<()> {
    writeln!(w, "[")?;
    for (i, r) in records.iter().enumerate() {
        let comma = if i + 1 < records.len() { "," } else { "" };
        writeln!(
            w,
            "  {{\"start\": {}, \"end\": {}, \"start_unix\": {}, \"end_unix\": {}, \"seconds\": {}, \"app\": {}, \"exe_path\": {}}}{comma}",
            json_str(&time(r.start)),
            json_str(&time(r.end)),
            r.start,
            r.end,
            r.end - r.start,
            json_str(&r.app),
            json_str(&r.exe_path),
        )?;
    }
    writeln!(w, "]")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(ts: i64) -> String {
        format!("T{ts}")
    }

    fn recs() -> Vec<Record> {
        vec![
            Record { start: 100, end: 160, app: "Firefox".into(), exe_path: r"C:\Program Files\Mozilla Firefox\firefox.exe".into() },
            Record { start: 200, end: 230, app: "Odd, \"quoted\" app".into(), exe_path: r"C:\a\b.exe".into() },
        ]
    }

    #[test]
    fn csv_has_header_rows_and_escaping() {
        let mut out = Vec::new();
        write_csv(&mut out, &recs(), fixed).unwrap();
        let s = String::from_utf8(out).unwrap();
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(lines[0], "start,end,seconds,app,exe_path");
        assert_eq!(lines[1], r"T100,T160,60,Firefox,C:\Program Files\Mozilla Firefox\firefox.exe");
        assert_eq!(lines[2], r#"T200,T230,30,"Odd, ""quoted"" app",C:\a\b.exe"#);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn json_is_valid_and_escaped() {
        let mut out = Vec::new();
        write_json(&mut out, &recs(), fixed).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("[\n") && s.trim_end().ends_with(']'));
        assert!(s.contains(r#""exe_path": "C:\\Program Files\\Mozilla Firefox\\firefox.exe""#));
        assert!(s.contains(r#""app": "Odd, \"quoted\" app""#));
        assert!(s.contains(r#""seconds": 60"#));
        assert_eq!(s.matches("},\n").count(), 1, "commas between records only");
    }

    #[test]
    fn empty_exports_are_well_formed() {
        let mut out = Vec::new();
        write_json(&mut out, &[], fixed).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "[\n]\n");
        let mut out = Vec::new();
        write_csv(&mut out, &[], fixed).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "start,end,seconds,app,exe_path\n");
    }

    #[test]
    fn records_come_from_the_store_with_friendly_names() {
        use crate::tracker::{Sample, Tracker};
        let mut s = Store::open_in_memory().unwrap();
        let a = s.app_id(r"C:\x\WindowsTerminal.exe", "Windows Terminal Host").unwrap();
        let mut t = Tracker::new(|ts| (ts / 86_400 + 1) * 86_400);
        for ts in 1000..1010 {
            t.tick(&Sample { ts, app: Some(a), ..Default::default() });
        }
        s.flush(&mut t).unwrap();
        let r = records(&s).unwrap();
        assert_eq!(r, vec![Record { start: 1000, end: 1010, app: "Terminal".into(), exe_path: r"C:\x\WindowsTerminal.exe".into() }]);
    }
}
