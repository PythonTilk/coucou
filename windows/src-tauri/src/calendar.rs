// Calendar pill: reads one or more ICS feeds (a university timetable, a
// calendar's public link, a Moodle export) and works out what is coming up.
//
// Only what a timetable needs: timed and all-day events, and the repeat rules
// they actually use — daily, weekly (with BYDAY), monthly and yearly, with
// INTERVAL, COUNT, UNTIL and EXDATE. Times are handled as local wall-clock
// time: a `Z` time is converted, and a TZID time is taken to be in this
// machine's zone, which is where its owner's lectures are.

use std::collections::HashSet;

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;

/// How far ahead the pill looks.
pub const HORIZON_DAYS: i64 = 14;
/// How many events it is told about.
pub const MAX_EVENTS: usize = 8;
/// An event that started this recently still counts as "now".
const GRACE_MINUTES: i64 = 15;
/// A repeat rule is never followed further than this many steps.
const MAX_STEPS: usize = 4000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Upcoming {
    /// Milliseconds since the epoch.
    pub start_ms: i64,
    pub title: String,
    pub location: Option<String>,
    pub all_day: bool,
}

/// One occurrence in local wall-clock time, before it becomes an instant.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub start: NaiveDateTime,
    pub title: String,
    pub location: Option<String>,
    pub all_day: bool,
}

/// What is coming up in these feeds, soonest first.
pub fn upcoming(feeds: &[String]) -> Vec<Upcoming> {
    let now = Local::now().naive_local();
    let mut all: Vec<Occurrence> = feeds
        .iter()
        .flat_map(|ics| occurrences(ics, now, &utc_to_local))
        .collect();
    all.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.title.cmp(&b.title)));
    all.dedup();
    all.into_iter()
        .take(MAX_EVENTS)
        .filter_map(|o| {
            let start = Local.from_local_datetime(&o.start).earliest()?;
            Some(Upcoming {
                start_ms: start.timestamp_millis(),
                title: o.title,
                location: o.location,
                all_day: o.all_day,
            })
        })
        .collect()
}

fn utc_to_local(utc: NaiveDateTime) -> NaiveDateTime {
    Utc.from_utc_datetime(&utc).with_timezone(&Local).naive_local()
}

/// Every occurrence of every event in `ics` between now and the horizon.
pub fn occurrences(
    ics: &str,
    now: NaiveDateTime,
    to_local: &dyn Fn(NaiveDateTime) -> NaiveDateTime,
) -> Vec<Occurrence> {
    let from_timed = now - Duration::minutes(GRACE_MINUTES);
    let today = now.date().and_hms_opt(0, 0, 0).unwrap_or(now);
    let until = now + Duration::days(HORIZON_DAYS);

    let mut out = Vec::new();
    for event in events(ics, to_local) {
        let from = if event.all_day { today } else { from_timed };
        for start in event.starts(from, until) {
            out.push(Occurrence {
                start,
                title: event.title.clone(),
                location: event.location.clone(),
                all_day: event.all_day,
            });
        }
    }
    out
}

// ── Events ───────────────────────────────────────────────────────────────────

struct Event {
    start: NaiveDateTime,
    all_day: bool,
    title: String,
    location: Option<String>,
    rule: Option<Rule>,
    excluded: HashSet<NaiveDateTime>,
}

#[derive(Debug, Clone, PartialEq)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Debug, Clone, PartialEq)]
struct Rule {
    freq: Freq,
    interval: i64,
    count: Option<usize>,
    until: Option<NaiveDateTime>,
    /// Weekdays as days from Monday (0–6). Empty: the start's own weekday.
    by_day: Vec<i64>,
}

impl Event {
    /// This event's starts within `[from, until]`, repeats followed.
    fn starts(&self, from: NaiveDateTime, until: NaiveDateTime) -> Vec<NaiveDateTime> {
        let Some(rule) = &self.rule else {
            return if self.start >= from && self.start <= until { vec![self.start] } else { Vec::new() };
        };

        let mut out = Vec::new();
        let mut produced = 0usize;
        // `false` ends the walk: past the window, the rule's end, or its count.
        let mut take = |start: NaiveDateTime| -> bool {
            if start > until || rule.until.is_some_and(|u| start > u) {
                return false;
            }
            if rule.count.is_some_and(|c| produced >= c) {
                return false;
            }
            produced += 1;
            if start >= from && !self.excluded.contains(&start) {
                out.push(start);
            }
            true
        };

        let time = self.start.time();
        let first = self.start.date();
        match rule.freq {
            Freq::Daily => {
                for step in 0..MAX_STEPS as i64 {
                    if !take((first + Duration::days(step * rule.interval)).and_time(time)) {
                        break;
                    }
                }
            }
            Freq::Weekly => {
                let mut days = rule.by_day.clone();
                if days.is_empty() {
                    days.push(first.weekday().num_days_from_monday() as i64);
                }
                days.sort_unstable();
                days.dedup();
                let monday = first - Duration::days(first.weekday().num_days_from_monday() as i64);
                'weeks: for week in 0..MAX_STEPS as i64 {
                    for day in &days {
                        let date = monday + Duration::days(week * rule.interval * 7 + day);
                        if date < first {
                            continue;
                        }
                        if !take(date.and_time(time)) {
                            break 'weeks;
                        }
                    }
                }
            }
            Freq::Monthly | Freq::Yearly => {
                let months = if rule.freq == Freq::Monthly { rule.interval } else { rule.interval * 12 };
                for step in 0..MAX_STEPS as i64 {
                    let total = first.year() as i64 * 12 + first.month0() as i64 + step * months;
                    let date = NaiveDate::from_ymd_opt(
                        total.div_euclid(12) as i32,
                        total.rem_euclid(12) as u32 + 1,
                        first.day(),
                    );
                    match date {
                        // The 31st of a month that has none: no occurrence, carry on.
                        None => {
                            if step > 0 && step * months > 12 * 400 {
                                break;
                            }
                        }
                        Some(date) => {
                            if !take(date.and_time(time)) {
                                break;
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

/// Parses every usable VEVENT. Cancelled events and ones without a start or a
/// title are left out.
fn events(ics: &str, to_local: &dyn Fn(NaiveDateTime) -> NaiveDateTime) -> Vec<Event> {
    let mut out = Vec::new();
    let mut current: Option<Vec<(String, String, String)>> = None;

    for line in unfold(ics) {
        let Some((name, params, value)) = split_line(&line) else { continue };
        match (name.as_str(), value.as_str()) {
            ("BEGIN", "VEVENT") => current = Some(Vec::new()),
            ("END", "VEVENT") => {
                if let Some(props) = current.take() {
                    if let Some(event) = build_event(&props, to_local) {
                        out.push(event);
                    }
                }
            }
            _ => {
                if let Some(props) = current.as_mut() {
                    props.push((name, params, value));
                }
            }
        }
    }
    out
}

fn build_event(
    props: &[(String, String, String)],
    to_local: &dyn Fn(NaiveDateTime) -> NaiveDateTime,
) -> Option<Event> {
    let get = |name: &str| props.iter().find(|p| p.0 == name);

    if get("STATUS").is_some_and(|p| p.2.eq_ignore_ascii_case("CANCELLED")) {
        return None;
    }
    let (_, _, raw_start) = get("DTSTART")?;
    let (start, all_day) = parse_time(raw_start, to_local)?;
    let title = unescape(&get("SUMMARY")?.2);
    if title.trim().is_empty() {
        return None;
    }
    let location = get("LOCATION").map(|p| unescape(&p.2)).filter(|l| !l.trim().is_empty());
    let rule = get("RRULE").and_then(|p| parse_rule(&p.2, to_local));
    let excluded = props
        .iter()
        .filter(|p| p.0 == "EXDATE")
        .flat_map(|p| p.2.split(','))
        .filter_map(|v| parse_time(v, to_local).map(|(t, _)| t))
        .collect();

    Some(Event { start, all_day, title: title.trim().to_string(), location, rule, excluded })
}

// ── Lines and values ─────────────────────────────────────────────────────────

/// A long line is folded over several, each continuation starting with a space
/// or a tab. This puts them back together.
fn unfold(ics: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.lines() {
        let raw = raw.trim_end_matches('\r');
        if let Some(rest) = raw.strip_prefix(' ').or_else(|| raw.strip_prefix('\t')) {
            if let Some(last) = lines.last_mut() {
                last.push_str(rest);
                continue;
            }
        }
        lines.push(raw.to_string());
    }
    lines
}

/// `NAME;PARAMS:VALUE` → (NAME, PARAMS, VALUE). A colon inside a quoted
/// parameter is not the separator.
fn split_line(line: &str) -> Option<(String, String, String)> {
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| match c {
        '"' => {
            quoted = !quoted;
            None
        }
        ':' if !quoted => Some(i),
        _ => None,
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let (name, params) = head.split_once(';').unwrap_or((head, ""));
    Some((name.trim().to_ascii_uppercase(), params.to_string(), value.trim().to_string()))
}

/// `20261005T141500Z`, `20261005T141500` or `20261005` → local time, and
/// whether it was a date only.
fn parse_time(
    value: &str,
    to_local: &dyn Fn(NaiveDateTime) -> NaiveDateTime,
) -> Option<(NaiveDateTime, bool)> {
    let value = value.trim();
    if value.len() == 8 {
        let date = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((date.and_hms_opt(0, 0, 0)?, true));
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let time = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
        return Some((to_local(time), false));
    }
    let time = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    Some((time, false))
}

fn parse_rule(value: &str, to_local: &dyn Fn(NaiveDateTime) -> NaiveDateTime) -> Option<Rule> {
    let mut freq = None;
    let mut rule = Rule { freq: Freq::Daily, interval: 1, count: None, until: None, by_day: Vec::new() };
    for part in value.split(';') {
        let Some((key, val)) = part.split_once('=') else { continue };
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = match val.to_ascii_uppercase().as_str() {
                    "DAILY" => Some(Freq::Daily),
                    "WEEKLY" => Some(Freq::Weekly),
                    "MONTHLY" => Some(Freq::Monthly),
                    "YEARLY" => Some(Freq::Yearly),
                    _ => None,
                }
            }
            "INTERVAL" => rule.interval = val.parse::<i64>().ok().filter(|n| *n > 0).unwrap_or(1),
            "COUNT" => rule.count = val.parse().ok(),
            // An all-day UNTIL means "through that day".
            "UNTIL" => {
                rule.until = parse_time(val, to_local).map(|(t, date_only)| {
                    if date_only { t + Duration::days(1) - Duration::seconds(1) } else { t }
                })
            }
            "BYDAY" => {
                rule.by_day = val
                    .split(',')
                    .filter_map(|d| {
                        // "MO", or "1MO" / "-1FR" in monthly rules: the weekday is the last two letters.
                        let d = d.trim();
                        let code = d.get(d.len().checked_sub(2)?..)?.to_ascii_uppercase();
                        ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
                            .iter()
                            .position(|w| *w == code)
                            .map(|i| i as i64)
                    })
                    .collect()
            }
            _ => {}
        }
    }
    rule.freq = freq?;
    Some(rule)
}

/// `\,` `\;` `\\` and `\n` as the format escapes them.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push(' '),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap()
    }

    /// Tests pretend the machine is two hours ahead of UTC.
    fn plus_two(utc: NaiveDateTime) -> NaiveDateTime {
        utc + Duration::hours(2)
    }

    fn feed(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{}\r\nEND:VCALENDAR\r\n", body.trim().replace('\n', "\r\n"))
    }

    fn starts(ics: &str, now: NaiveDateTime) -> Vec<(NaiveDateTime, String)> {
        let mut all = occurrences(ics, now, &plus_two);
        all.sort_by(|a, b| a.start.cmp(&b.start));
        all.into_iter().map(|o| (o.start, o.title)).collect()
    }

    // Monday 5 October 2026, 09:00.
    fn monday() -> NaiveDateTime {
        at(2026, 10, 5, 9, 0)
    }

    #[test]
    fn a_single_event_is_listed_with_its_place() {
        let ics = feed(
            "BEGIN:VEVENT\nDTSTART;TZID=Europe/Berlin:20261006T141500\nSUMMARY:Analysis II\nLOCATION:HS 3\\, Main building\nEND:VEVENT",
        );
        let all = occurrences(&ics, monday(), &plus_two);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].start, at(2026, 10, 6, 14, 15));
        assert_eq!(all[0].title, "Analysis II");
        assert_eq!(all[0].location.as_deref(), Some("HS 3, Main building"));
        assert!(!all[0].all_day);
    }

    #[test]
    fn a_utc_time_is_shown_in_local_time() {
        let ics = feed("BEGIN:VEVENT\nDTSTART:20261006T121500Z\nSUMMARY:Seminar\nEND:VEVENT");
        assert_eq!(starts(&ics, monday()), vec![(at(2026, 10, 6, 14, 15), "Seminar".to_string())]);
    }

    #[test]
    fn what_is_over_or_too_far_ahead_is_left_out() {
        let ics = feed(
            "BEGIN:VEVENT\nDTSTART:20261005T080000\nSUMMARY:Earlier today\nEND:VEVENT\n\
             BEGIN:VEVENT\nDTSTART:20261005T085500\nSUMMARY:Just started\nEND:VEVENT\n\
             BEGIN:VEVENT\nDTSTART:20261101T100000\nSUMMARY:Next month\nEND:VEVENT",
        );
        assert_eq!(starts(&ics, monday()), vec![(at(2026, 10, 5, 8, 55), "Just started".to_string())]);
    }

    #[test]
    fn a_deadline_today_stays_all_day() {
        let ics = feed("BEGIN:VEVENT\nDTSTART;VALUE=DATE:20261005\nSUMMARY:Hand in sheet 3\nEND:VEVENT");
        let all = occurrences(&ics, at(2026, 10, 5, 18, 0), &plus_two);
        assert_eq!(all.len(), 1);
        assert!(all[0].all_day);
        assert_eq!(all[0].start, at(2026, 10, 5, 0, 0));
    }

    #[test]
    fn a_weekly_lecture_repeats_on_its_days() {
        // Started weeks ago, Mondays and Thursdays.
        let ics = feed(
            "BEGIN:VEVENT\nDTSTART:20260907T101500\nRRULE:FREQ=WEEKLY;BYDAY=MO,TH\nSUMMARY:Linear Algebra\nEND:VEVENT",
        );
        let days: Vec<NaiveDateTime> = starts(&ics, monday()).into_iter().map(|s| s.0).collect();
        assert_eq!(
            days,
            vec![
                at(2026, 10, 5, 10, 15),
                at(2026, 10, 8, 10, 15),
                at(2026, 10, 12, 10, 15),
                at(2026, 10, 15, 10, 15),
                // 19 October 10:15 is just past the 14-day horizon (19 October 09:00).
            ]
        );
    }

    #[test]
    fn a_fortnightly_rule_skips_the_weeks_between() {
        let ics = feed(
            "BEGIN:VEVENT\nDTSTART:20260928T160000\nRRULE:FREQ=WEEKLY;INTERVAL=2\nSUMMARY:Tutorial\nEND:VEVENT",
        );
        let days: Vec<NaiveDateTime> = starts(&ics, monday()).into_iter().map(|s| s.0).collect();
        assert_eq!(days, vec![at(2026, 10, 12, 16, 0)]);
    }

    #[test]
    fn a_rule_stops_at_its_count_or_its_end() {
        let counted = feed(
            "BEGIN:VEVENT\nDTSTART:20260928T090000\nRRULE:FREQ=DAILY;COUNT=9\nSUMMARY:Block course\nEND:VEVENT",
        );
        // 28 Sept + 8 more days ends on 6 Oct; from Monday 5 Oct 09:00 that leaves two.
        assert_eq!(starts(&counted, monday()).len(), 2);

        let ended = feed(
            "BEGIN:VEVENT\nDTSTART:20260907T140000\nRRULE:FREQ=WEEKLY;UNTIL=20261012\nSUMMARY:Lab\nEND:VEVENT",
        );
        let days: Vec<NaiveDateTime> = starts(&ended, monday()).into_iter().map(|s| s.0).collect();
        assert_eq!(days, vec![at(2026, 10, 5, 14, 0), at(2026, 10, 12, 14, 0)]);
    }

    #[test]
    fn a_cancelled_week_is_skipped() {
        let ics = feed(
            "BEGIN:VEVENT\nDTSTART:20260907T140000\nRRULE:FREQ=WEEKLY\nEXDATE:20261012T140000\nSUMMARY:Lab\nEND:VEVENT",
        );
        let days: Vec<NaiveDateTime> = starts(&ics, monday()).into_iter().map(|s| s.0).collect();
        // 12 October is excluded, and it shows: without the EXDATE it would be here.
        assert_eq!(days, vec![at(2026, 10, 5, 14, 0)]);
        let kept = feed(
            "BEGIN:VEVENT
DTSTART:20260907T140000
RRULE:FREQ=WEEKLY
SUMMARY:Lab
END:VEVENT",
        );
        assert_eq!(starts(&kept, monday()).len(), 2);
    }

    #[test]
    fn folded_lines_and_cancelled_events_are_handled() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:20261006T100000\r\nSUMMARY:Introduction to \r\n Computer Science\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nDTSTART:20261006T120000\r\nSTATUS:CANCELLED\r\nSUMMARY:Gone\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        assert_eq!(
            starts(ics, monday()),
            vec![(at(2026, 10, 6, 10, 0), "Introduction to Computer Science".to_string())]
        );
    }

    #[test]
    fn a_monthly_rule_on_the_31st_skips_short_months() {
        let ics = feed(
            "BEGIN:VEVENT\nDTSTART:20260831T120000\nRRULE:FREQ=MONTHLY\nSUMMARY:Rent\nEND:VEVENT",
        );
        // September has no 31st; within 14 days of 20 October that leaves 31 October.
        let days: Vec<NaiveDateTime> =
            starts(&ics, at(2026, 10, 20, 9, 0)).into_iter().map(|s| s.0).collect();
        assert_eq!(days, vec![at(2026, 10, 31, 12, 0)]);
    }

    #[test]
    fn rubbish_is_not_a_calendar() {
        assert!(occurrences("<html>Sign in</html>", monday(), &plus_two).is_empty());
        assert!(occurrences("", monday(), &plus_two).is_empty());
    }
}
