//! `screentime` — command line front end.

use std::path::PathBuf;
use std::process::ExitCode;

use chrono::{Datelike, Days, NaiveDate};
use screentime_core::config::Config;
use screentime_core::export::{self, Format};
use screentime_core::report::{self, Catalog, Usage};
use screentime_core::store::{self, Store};
use screentime_core::time::{fmt_duration, now_ts, today};

#[cfg(windows)]
use screentime_core::win;

const TRACKER_EXE: &str = "screentime-tracker.exe";
const RULE_WIDTH: usize = 64;

const HELP: &str = "\
screentime — local screen time tracker for Windows

USAGE
  screentime run              start tracking in the background
  screentime stop             stop the background tracker
  screentime status           tracker, database and startup status
  screentime today            time per app today
  screentime week             last 7 days, per day, plus top apps
  screentime startup on|off   start tracking when you log in (current user)
  screentime pause | resume   pause or resume tracking (the tracker keeps running)
  screentime export --csv|--json [FILE]
                              every session, to FILE or the console

Data stays on this PC: %LOCALAPPDATA%\\screentime\\data.db
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["run"] => cmd_run(),
        ["stop"] => cmd_stop(),
        ["status"] => cmd_status(),
        ["today"] => cmd_today(),
        ["week"] => cmd_week(),
        ["startup"] => cmd_startup(None),
        ["startup", v @ ("on" | "off")] => cmd_startup(Some(*v == "on")),
        ["pause"] => cmd_pause(true),
        ["resume"] => cmd_pause(false),
        ["export", fmt, rest @ ..] if rest.len() <= 1 => match *fmt {
            "--csv" => cmd_export(Format::Csv, rest.first().copied()),
            "--json" => cmd_export(Format::Json, rest.first().copied()),
            _ => Err(format!("export needs --csv or --json

{HELP}")),
        },
        [] | ["help" | "-h" | "--help"] => {
            print!("{HELP}");
            Ok(())
        }
        _ => Err(format!("unknown command: {}\n\n{HELP}", args.join(" "))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

type Res = Result<(), String>;

// ------------------------------------------------------------- tracker control

fn tracker_exe() -> Result<PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = me.with_file_name(TRACKER_EXE);
    if exe.exists() {
        Ok(exe)
    } else {
        Err(format!("{TRACKER_EXE} not found next to {}", me.display()))
    }
}

#[cfg(windows)]
fn cmd_run() -> Res {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    if win::tracker_running() {
        println!("Already tracking.");
        return Ok(());
    }
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

    let exe = tracker_exe()?;
    win::disinherit_std_handles();
    let spawn = |flags: u32| {
        Command::new(&exe)
            .creation_flags(flags)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    };
    // Outlive the terminal that started us. Some terminals put children in a
    // job that forbids breakaway; then fall back to a plain detached start.
    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    let mut child = spawn(base | CREATE_BREAKAWAY_FROM_JOB)
        .or_else(|_| spawn(base))
        .map_err(|e| format!("cannot start {}: {e}", exe.display()))?;

    for _ in 0..30 {
        if win::tracker_running() {
            println!("Tracking in the background (pid {}).", child.id());
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "tracker exited at startup ({status}); see {}",
                store::data_dir().join("tracker.log").display()
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err("tracker did not start within 3 s".into())
}

#[cfg(windows)]
fn cmd_stop() -> Res {
    if !win::signal_stop() {
        println!("Tracker is not running.");
        return Ok(());
    }
    for _ in 0..50 {
        if !win::tracker_running() {
            println!("Tracker stopped. Data saved.");
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Err("tracker did not stop within 5 s".into())
}

#[cfg(windows)]
fn cmd_status() -> Res {
    let running = match (win::tracker_running(), Config::load().paused) {
        (true, false) => "running",
        (true, true) => "running, paused (screentime resume)",
        (false, _) => "not running",
    };
    let startup = match win::get_startup() {
        Some(cmd) => format!("on  ({cmd})"),
        None => "off".into(),
    };
    println!("  tracker   {running}");
    println!("  startup   {startup}");
    println!("  database  {}", store::default_db_path().display());
    if let Some(s) = open_store()? {
        if let Ok(Some(ts)) = s.last_seen() {
            let ago = now_ts() - ts;
            println!("  last data {} ago", fmt_duration(ago));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn cmd_startup(enable: Option<bool>) -> Res {
    match enable {
        Some(true) => {
            let cmd = format!("\"{}\"", tracker_exe()?.display());
            win::set_startup(&cmd).map_err(|e| format!("cannot write Run key: {e}"))?;
            println!("Screentime will start when you log in.\n  {cmd}");
        }
        Some(false) => {
            win::clear_startup().map_err(|e| format!("cannot remove Run key: {e}"))?;
            println!("Screentime will no longer start when you log in.");
        }
        None => match win::get_startup() {
            Some(cmd) => println!("Startup is on.\n  {cmd}"),
            None => println!("Startup is off. Turn it on with: screentime startup on"),
        },
    }
    Ok(())
}

#[cfg(not(windows))]
fn cmd_run() -> Res {
    Err("tracking is only supported on Windows".into())
}
#[cfg(not(windows))]
fn cmd_stop() -> Res {
    cmd_run()
}
#[cfg(not(windows))]
fn cmd_status() -> Res {
    cmd_run()
}
#[cfg(not(windows))]
fn cmd_startup(_: Option<bool>) -> Res {
    cmd_run()
}

fn tracker_note() -> &'static str {
    #[cfg(windows)]
    if !win::tracker_running() {
        return "  tracker is not running — start it with: screentime run";
    }
    ""
}

// --------------------------------------------------------------------- reports

fn open_store() -> Result<Option<Store>, String> {
    let path = store::default_db_path();
    Store::open_read_only(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))
}

fn no_data() -> Res {
    println!("No data yet. Start tracking with: screentime run");
    Ok(())
}

fn cmd_pause(paused: bool) -> Res {
    let config = Config { paused, ..Config::load() };
    config.save().map_err(|e| format!("cannot save settings: {e}"))?;
    #[cfg(windows)]
    win::signal_reload();
    println!("{}", if paused { "Tracking paused. Resume with: screentime resume" } else { "Tracking resumed." });
    Ok(())
}

fn cmd_export(format: Format, file: Option<&str>) -> Res {
    let Some(store) = open_store()? else { return no_data() };
    let records = export::records(&store).map_err(|e| e.to_string())?;
    let result = match file {
        Some(path) => std::fs::File::create(path)
            .and_then(|f| {
                let mut w = std::io::BufWriter::new(f);
                export::write(&mut w, format, &records)?;
                std::io::Write::flush(&mut w)
            })
            .map(|_| eprintln!("Exported {} sessions to {path}", records.len())),
        None => export::write(&mut std::io::stdout().lock(), format, &records),
    };
    result.map_err(|e| format!("export failed: {e}"))
}

fn catalog(store: &Store) -> Result<Catalog, String> {
    Catalog::load(store).map_err(|e| e.to_string())
}

fn cmd_today() -> Res {
    let Some(store) = open_store()? else { return no_data() };
    let cat = catalog(&store)?;
    let date = today();
    let day = report::day(&store, &cat, date, None).map_err(|e| e.to_string())?;
    let (apps, total) = (day.apps, day.total);

    header(&format!("Today · {}", date.format("%a %-d %b")), &fmt_duration(total));
    if apps.is_empty() {
        println!("  Nothing tracked yet today.");
    } else {
        app_table(&apps, total, None);
    }
    footer(tracker_note());
    Ok(())
}

fn cmd_week() -> Res {
    let Some(store) = open_store()? else { return no_data() };
    let last = today();
    let first = last.checked_sub_days(Days::new(6)).unwrap_or(last);
    let cat = catalog(&store)?;
    let dates: Vec<NaiveDate> = first.iter_days().take(7).collect();
    let totals = report::day_totals(&store, &cat, &dates).map_err(|e| e.to_string())?;
    let days: Vec<(NaiveDate, i64)> = dates.into_iter().zip(totals).collect();
    let total: i64 = days.iter().map(|d| d.1).sum();
    let tracked_days = days.iter().filter(|d| d.1 > 0).count().max(1) as i64;

    let title = format!("Last 7 days · {} – {}", first.format("%-d %b"), last.format("%-d %b"));
    let summary = format!("{}  ·  avg {} per tracked day", fmt_duration(total), fmt_duration(total / tracked_days));
    header(&title, &summary);

    let max = days.iter().map(|d| d.1).max().unwrap_or(0);
    for (d, secs) in &days {
        let label = format!("{} {:>2}", d.format("%a"), d.day());
        let time = if *secs > 0 { fmt_duration(*secs) } else { "–".into() };
        let bar = bar(*secs, max, 36);
        let mark = if *d == last { format!("{}  today", pad(&bar, 36)) } else { bar };
        println!("{}", format!("  {label}  {time:>7}  {mark}").trim_end());
    }

    let apps = report::range_usage(&store, &cat, first, last).map_err(|e| e.to_string())?;
    println!();
    println!("  Top apps");
    if apps.is_empty() {
        println!("  Nothing tracked this week.");
    } else {
        app_table(&apps, total, Some(5));
    }
    footer(tracker_note());
    Ok(())
}

// ------------------------------------------------------------------ rendering

fn header(title: &str, right: &str) {
    let gap = RULE_WIDTH.saturating_sub(width(title) + width(right)).max(2);
    println!();
    println!("  {title}{}{right}", " ".repeat(gap));
    println!("  {}", "─".repeat(RULE_WIDTH));
}

fn footer(note: &str) {
    if !note.is_empty() {
        println!();
        println!("{note}");
    }
    println!();
}

fn app_table(apps: &[Usage], total: i64, limit: Option<usize>) {
    const NAME_MAX: usize = 26;
    let shown = &apps[..limit.unwrap_or(apps.len()).min(apps.len())];
    let name_w = shown.iter().map(|a| width(&a.name).min(NAME_MAX)).max().unwrap_or(0);
    let max = shown.first().map_or(0, |a| a.secs);
    let bar_w = RULE_WIDTH.saturating_sub(name_w + 7 + 5 + 6).clamp(10, 36);
    for (i, a) in shown.iter().enumerate() {
        let rank = if limit.is_some() { format!("{}. ", i + 1) } else { String::new() };
        let pct = if total > 0 { a.secs * 100 / total } else { 0 };
        println!(
            "  {rank}{}  {:>7}  {}  {pct:>3}%",
            pad(&truncate(&a.name, NAME_MAX), name_w),
            fmt_duration(a.secs),
            pad(&bar(a.secs, max, bar_w), bar_w),
        );
    }
    if limit.is_none() && apps.len() > shown.len() {
        println!("  … and {} more", apps.len() - shown.len());
    }
}

/// Horizontal bar with 1/8-cell resolution.
fn bar(value: i64, max: i64, cells: usize) -> String {
    const PARTS: [char; 8] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉'];
    if max <= 0 || value <= 0 {
        return String::new();
    }
    let eighths = ((value as f64 / max as f64) * (cells * 8) as f64).round().max(1.0) as usize;
    let mut s = "█".repeat(eighths / 8);
    if !eighths.is_multiple_of(8) {
        s.push(PARTS[eighths % 8]);
    }
    s
}

fn width(s: &str) -> usize {
    s.chars().count()
}

fn pad(s: &str, w: usize) -> String {
    format!("{s}{}", " ".repeat(w.saturating_sub(width(s))))
}

fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_scale_with_eighth_precision() {
        assert_eq!(bar(0, 100, 10), "");
        assert_eq!(bar(100, 100, 10), "█".repeat(10));
        assert_eq!(bar(50, 100, 10), "█".repeat(5));
        assert_eq!(bar(1, 1000, 10), "▏"); // tiny but visible
        assert_eq!(bar(55, 100, 10), "█████▌");
    }

    #[test]
    fn long_names_are_truncated() {
        assert_eq!(truncate("Notepad", 26), "Notepad");
        assert_eq!(width(&truncate(&"x".repeat(40), 26)), 26);
    }
}
