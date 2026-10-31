//! 캘린더와 미리 알림(EventKit, macOS). 녹음 시각과 겹치는 일정을 찾아 제목과 참석자를 AI 정리에 넘기고,
//! 사용자가 확인한 할 일을 미리 알림의 기본 목록에 넣는다. 다른 OS는 일정이 없고 할 일을 넣지 못한다.
//! @see docs/organize.md

use chrono::{DateTime, Utc};
use eyre::Result;

use crate::store::{CalendarEvent, Todo};

/// 일정 앞뒤로 이만큼 늦게 시작하거나 일찍 시작한 녹음도 같은 일정으로 본다
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SLACK_MINUTES: i64 = 10;

/// 녹음 구간과 가장 많이 겹치는 일정. 종일 일정은 뺀다. 캘린더를 허락하지 않았으면 None
#[cfg(target_os = "macos")]
pub fn event_for(start: DateTime<Utc>, end: DateTime<Utc>) -> Option<CalendarEvent> {
    mac::event_for(start, end)
}

#[cfg(not(target_os = "macos"))]
pub fn event_for(_: DateTime<Utc>, _: DateTime<Utc>) -> Option<CalendarEvent> {
    None
}

/// 할 일을 미리 알림의 기본 목록에 넣는다. 넣은 개수를 돌려준다. 처음에는 macOS가 허락을 묻는다
#[cfg(target_os = "macos")]
pub fn add_reminders(todos: &[Todo], note: &str) -> Result<usize> {
    mac::add_reminders(todos, note)
}

#[cfg(not(target_os = "macos"))]
pub fn add_reminders(_: &[Todo], _: &str) -> Result<usize> {
    eyre::bail!("미리 알림은 macOS에서만 넣을 수 있습니다")
}

/// 녹음 구간과 겹치는 시간(초). 겹치지 않으면 0
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn overlap(a: (DateTime<Utc>, DateTime<Utc>), b: (DateTime<Utc>, DateTime<Utc>)) -> i64 {
    let start = a.0.max(b.0);
    let end = a.1.min(b.1);
    (end - start).num_seconds().max(0)
}

/// 겹치는 시간이 가장 긴 것. 녹음이 일정 바로 앞뒤(SLACK_MINUTES 안)에 걸쳐도 고른다
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn best_match(recording: (DateTime<Utc>, DateTime<Utc>), events: Vec<CalendarEvent>) -> Option<CalendarEvent> {
    let slack = chrono::Duration::minutes(SLACK_MINUTES);
    let widened = (recording.0 - slack, recording.1 + slack);
    events
        .into_iter()
        .map(|e| (overlap(widened, (e.start, e.end)), e))
        .filter(|(o, _)| *o > 0)
        .max_by_key(|(o, _)| *o)
        .map(|(_, e)| e)
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::mpsc;
    use std::time::Duration;

    use block2::RcBlock;
    use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
    use eyre::{eyre, Result};
    use objc2::runtime::Bool;
    use objc2_event_kit::{EKAuthorizationStatus, EKEntityType, EKEventStore, EKReminder};
    use objc2_foundation::{NSDate, NSDateComponents, NSError, NSString};

    use crate::store::{CalendarEvent, Todo};

    /// 사용자가 허락 창에 답할 때까지 기다리는 시간
    const ASK_TIMEOUT: Duration = Duration::from_secs(120);

    fn to_ns(t: DateTime<Utc>) -> objc2::rc::Retained<NSDate> {
        NSDate::dateWithTimeIntervalSince1970(t.timestamp() as f64)
    }

    fn from_ns(d: &NSDate) -> DateTime<Utc> {
        Utc.timestamp_opt(d.timeIntervalSince1970() as i64, 0).single().unwrap_or_else(Utc::now)
    }

    /// 허락을 받았는지. 처음이면 묻고 답을 기다린다
    fn access(store: &EKEventStore, kind: EKEntityType) -> bool {
        let status = unsafe { EKEventStore::authorizationStatusForEntityType(kind) };
        if status == EKAuthorizationStatus::FullAccess {
            return true;
        }
        if status != EKAuthorizationStatus::NotDetermined {
            return false;
        }
        let (tx, rx) = mpsc::channel::<bool>();
        let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            let _ = tx.send(granted.as_bool());
        });
        unsafe {
            if kind == EKEntityType::Reminder {
                store.requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&block));
            } else {
                store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block));
            }
        }
        rx.recv_timeout(ASK_TIMEOUT).unwrap_or(false)
    }

    pub fn event_for(start: DateTime<Utc>, end: DateTime<Utc>) -> Option<CalendarEvent> {
        let store = unsafe { EKEventStore::new() };
        if !access(&store, EKEntityType::Event) {
            return None;
        }
        let slack = chrono::Duration::minutes(super::SLACK_MINUTES);
        let events = unsafe {
            let predicate = store.predicateForEventsWithStartDate_endDate_calendars(&to_ns(start - slack), &to_ns(end + slack), None);
            store.eventsMatchingPredicate(&predicate)
        };
        let candidates: Vec<CalendarEvent> = events
            .iter()
            .filter(|e| unsafe { !e.isAllDay() })
            .map(|e| unsafe {
                let attendees = e
                    .attendees()
                    .map(|list| {
                        list.iter()
                            .filter(|p| !p.isCurrentUser())
                            .filter_map(|p| p.name().map(|n| n.to_string()))
                            .filter(|n| !n.trim().is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                CalendarEvent {
                    title: e.title().to_string(),
                    start: from_ns(&e.startDate()),
                    end: from_ns(&e.endDate()),
                    attendees,
                    calendar: e.calendar().map(|c| c.title().to_string()),
                }
            })
            .filter(|e| !e.title.trim().is_empty())
            .collect();
        super::best_match((start, end), candidates)
    }

    pub fn add_reminders(todos: &[Todo], note: &str) -> Result<usize> {
        let store = unsafe { EKEventStore::new() };
        if !access(&store, EKEntityType::Reminder) {
            eyre::bail!("미리 알림에 접근할 수 없습니다. 시스템 설정 > 개인정보 보호 및 보안 > 미리 알림에서 sori를 허용해 주세요");
        }
        let calendar = unsafe { store.defaultCalendarForNewReminders() }.ok_or_else(|| eyre!("미리 알림의 기본 목록이 없습니다"))?;
        let mut added = 0;
        for todo in todos {
            unsafe {
                let reminder = EKReminder::reminderWithEventStore(&store);
                reminder.setTitle(Some(&NSString::from_str(&todo.text)));
                reminder.setNotes(Some(&NSString::from_str(note)));
                reminder.setCalendar(Some(&calendar));
                if let Some(due) = todo.due.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
                    let parts = NSDateComponents::new();
                    parts.setYear(due.year() as isize);
                    parts.setMonth(due.month() as isize);
                    parts.setDay(due.day() as isize);
                    reminder.setDueDateComponents(Some(&parts));
                }
                store.saveReminder_commit_error(&reminder, false).map_err(|e| eyre!("미리 알림에 넣지 못했습니다: {}", e.localizedDescription()))?;
            }
            added += 1;
        }
        unsafe { store.commit() }.map_err(|e| eyre!("미리 알림에 저장하지 못했습니다: {}", e.localizedDescription()))?;
        Ok(added)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(h: u32, m: u32) -> DateTime<Utc> {
        chrono::TimeZone::with_ymd_and_hms(&Utc, 2026, 10, 31, h, m, 0).unwrap()
    }

    fn event(title: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> CalendarEvent {
        CalendarEvent { title: title.into(), start, end, attendees: Vec::new(), calendar: None }
    }

    #[test]
    fn 가장_많이_겹치는_일정을_고른다() {
        let found = best_match(
            (at(10, 0), at(11, 0)),
            vec![event("짧게 겹침", at(9, 0), at(10, 10)), event("주간 회의", at(10, 0), at(11, 0))],
        );
        assert_eq!(found.unwrap().title, "주간 회의");
    }

    #[test]
    fn 일정_직전에_시작한_녹음도_그_일정으로_본다() {
        let found = best_match((at(9, 55), at(9, 58)), vec![event("고객 미팅", at(10, 0), at(11, 0))]);
        assert_eq!(found.unwrap().title, "고객 미팅");
    }

    #[test]
    fn 멀리_떨어진_일정은_고르지_않는다() {
        assert!(best_match((at(13, 0), at(14, 0)), vec![event("오전 회의", at(10, 0), at(11, 0))]).is_none());
    }
}
