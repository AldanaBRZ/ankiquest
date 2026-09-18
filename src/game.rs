use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

const SESSION_GAP_MS: i64 = 300_000;
const MATURE_IVL: i64 = 21;
const MAX_FREEZES: u32 = 2;
const FREEZE_EVERY: u32 = 7;
const QUEST_XP: u64 = 50;
const ALL_QUESTS_XP: u64 = 100;
const RECENT_DAYS: usize = 14;
const HEATMAP_DAYS: i64 = 182;

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Review {
    pub id: i64,
    pub cid: i64,
    pub last_ivl: i64,
    pub time_ms: i64,
    pub kind: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Clock {
    pub offset_west_min: i64,
    pub rollover_hour: i64,
}

impl Default for Clock {
    fn default() -> Self {
        Self {
            offset_west_min: 0,
            rollover_hour: 4,
        }
    }
}

impl Clock {
    fn local_secs(&self, ms: i64) -> i64 {
        ms.div_euclid(1000) - self.offset_west_min * 60
    }

    pub fn day(&self, ms: i64) -> i64 {
        (self.local_secs(ms) - self.rollover_hour * 3600).div_euclid(86400)
    }

    pub fn hour(&self, ms: i64) -> i64 {
        self.local_secs(ms).rem_euclid(86400) / 3600
    }
}

#[derive(Clone, Default, Debug)]
struct DayStats {
    reviews: u64,
    new_cards: u64,
    mature: u64,
    time_ms: i64,
    max_combo: u64,
    sessions: u64,
    before_noon: u64,
    early: bool,
    late: bool,
    review_xp: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuestKind {
    Reviews,
    Minutes,
    Combo,
    Mature,
    NewCards,
    Sessions,
    BeforeNoon,
}

#[derive(Clone, Copy, Debug)]
struct Quest {
    kind: QuestKind,
    target: u64,
}

impl Quest {
    fn progress(&self, s: &DayStats) -> u64 {
        match self.kind {
            QuestKind::Reviews => s.reviews,
            QuestKind::Minutes => (s.time_ms / 60_000) as u64,
            QuestKind::Combo => s.max_combo,
            QuestKind::Mature => s.mature,
            QuestKind::NewCards => s.new_cards,
            QuestKind::Sessions => s.sessions,
            QuestKind::BeforeNoon => s.before_noon,
        }
    }

    fn title(&self) -> String {
        let n = self.target;
        match self.kind {
            QuestKind::Reviews => format!("Review {n} cards"),
            QuestKind::Minutes => format!("Study for {n} minutes"),
            QuestKind::Combo => format!("Reach a {n} card combo"),
            QuestKind::Mature => format!("Review {n} mature cards"),
            QuestKind::NewCards => format!("Learn {n} new cards"),
            QuestKind::Sessions => format!("Study in {n} separate sessions"),
            QuestKind::BeforeNoon => format!("Review {n} cards before noon"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Metric {
    Reviews,
    Streak,
    DayReviews,
    Combo,
    Hours,
    Mature,
    NewCards,
    Quests,
    PerfectDays,
    EarlyDays,
    LateDays,
}

struct Def {
    metric: Metric,
    threshold: u64,
    title: &'static str,
}

const fn def(metric: Metric, threshold: u64, title: &'static str) -> Def {
    Def {
        metric,
        threshold,
        title,
    }
}

const ACHIEVEMENTS: &[Def] = &[
    def(Metric::Reviews, 100, "First Hundred"),
    def(Metric::Reviews, 1_000, "Thousand Flips"),
    def(Metric::Reviews, 5_000, "Card Shark"),
    def(Metric::Reviews, 10_000, "Ten Thousand Hours, Sort Of"),
    def(Metric::Reviews, 25_000, "Memory Palace"),
    def(Metric::Reviews, 50_000, "Living Library"),
    def(Metric::Reviews, 100_000, "Mnemosyne"),
    def(Metric::Streak, 3, "Warming Up"),
    def(Metric::Streak, 7, "Full Week"),
    def(Metric::Streak, 14, "Fortnight"),
    def(Metric::Streak, 30, "Month of Mondays"),
    def(Metric::Streak, 60, "Habit Formed"),
    def(Metric::Streak, 100, "Centurion"),
    def(Metric::Streak, 200, "Unbreakable"),
    def(Metric::Streak, 365, "Orbit Complete"),
    def(Metric::DayReviews, 100, "Busy Day"),
    def(Metric::DayReviews, 250, "Marathon"),
    def(Metric::DayReviews, 500, "Ultramarathon"),
    def(Metric::Combo, 50, "In the Zone"),
    def(Metric::Combo, 100, "Flow State"),
    def(Metric::Combo, 250, "Trance"),
    def(Metric::Hours, 10, "Ten Hours In"),
    def(Metric::Hours, 50, "Fifty Hours In"),
    def(Metric::Hours, 100, "Hundred Hours In"),
    def(Metric::Hours, 500, "Scholar"),
    def(Metric::Mature, 100, "Long Term"),
    def(Metric::Mature, 1_000, "Deep Roots"),
    def(Metric::Mature, 10_000, "Old Growth"),
    def(Metric::NewCards, 100, "Collector"),
    def(Metric::NewCards, 1_000, "Curator"),
    def(Metric::NewCards, 5_000, "Archivist"),
    def(Metric::Quests, 10, "Adventurer"),
    def(Metric::Quests, 50, "Quest Hound"),
    def(Metric::Quests, 200, "Completionist"),
    def(Metric::PerfectDays, 7, "Perfect Week"),
    def(Metric::PerfectDays, 30, "Perfect Month"),
    def(Metric::EarlyDays, 5, "Early Bird"),
    def(Metric::LateDays, 5, "Night Owl"),
];

#[derive(Default)]
struct Totals {
    reviews: u64,
    best_streak: u64,
    best_day: u64,
    best_combo: u64,
    time_ms: i64,
    mature: u64,
    new_cards: u64,
    quests: u64,
    perfect_days: u64,
    early_days: u64,
    late_days: u64,
    days_active: u64,
}

impl Totals {
    fn value(&self, metric: Metric) -> u64 {
        match metric {
            Metric::Reviews => self.reviews,
            Metric::Streak => self.best_streak,
            Metric::DayReviews => self.best_day,
            Metric::Combo => self.best_combo,
            Metric::Hours => (self.time_ms / 3_600_000) as u64,
            Metric::Mature => self.mature,
            Metric::NewCards => self.new_cards,
            Metric::Quests => self.quests,
            Metric::PerfectDays => self.perfect_days,
            Metric::EarlyDays => self.early_days,
            Metric::LateDays => self.late_days,
        }
    }
}

fn describe(metric: Metric, n: u64) -> String {
    match metric {
        Metric::Reviews => format!("Review {n} cards in total"),
        Metric::Streak => format!("Reach a {n} day streak"),
        Metric::DayReviews => format!("Review {n} cards in one day"),
        Metric::Combo => format!("Reach a {n} card combo"),
        Metric::Hours => format!("Study for {n} hours in total"),
        Metric::Mature => format!("Review {n} mature cards"),
        Metric::NewCards => format!("Learn {n} new cards"),
        Metric::Quests => format!("Complete {n} quests"),
        Metric::PerfectDays => format!("Complete every quest on {n} days"),
        Metric::EarlyDays => format!("Study before 7am on {n} days"),
        Metric::LateDays => format!("Study after 11pm on {n} days"),
    }
}

fn achievement_xp(metric: Metric, threshold: u64) -> u64 {
    let tier = ACHIEVEMENTS
        .iter()
        .filter(|d| d.metric as u8 == metric as u8 && d.threshold <= threshold)
        .count() as u64;
    100 * tier
}

#[derive(Serialize, Clone, Debug)]
pub struct QuestView {
    pub title: String,
    pub progress: u64,
    pub target: u64,
    pub done: bool,
    pub reward: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct AchievementView {
    pub id: String,
    pub title: &'static str,
    pub description: String,
    pub reward: u64,
    pub progress: u64,
    pub target: u64,
    pub unlocked: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct HeatCell {
    pub date: String,
    pub reviews: u64,
    pub xp: u64,
    pub frozen: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Today {
    pub reviews: u64,
    pub minutes: u64,
    pub xp: u64,
    pub max_combo: u64,
    pub current_combo: u64,
    pub new_cards: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Lifetime {
    pub reviews: u64,
    pub hours: u64,
    pub days_active: u64,
    pub best_streak: u64,
    pub best_day: u64,
    pub best_combo: u64,
    pub quests: u64,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub key: String,
    pub title: String,
    pub body: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Profile {
    pub user: String,
    pub display: String,
    pub level: u64,
    pub xp_total: u64,
    pub xp_into_level: u64,
    pub xp_for_next: u64,
    pub week_xp: u64,
    pub streak: u64,
    pub freezes: u32,
    pub at_risk: bool,
    pub local_hour: i64,
    pub today: Today,
    pub lifetime: Lifetime,
    pub quests: Vec<QuestView>,
    pub achievements: Vec<AchievementView>,
    pub heatmap: Vec<HeatCell>,
    pub last_review_id: i64,
    #[serde(skip)]
    pub day: i64,
    #[serde(skip)]
    pub events: Vec<Event>,
}

pub fn level_need(level: u64) -> u64 {
    ((level as f64).powf(1.5) * 10.0).round() as u64 * 10
}

pub fn level_for(xp: u64) -> (u64, u64, u64) {
    let mut level = 1;
    let mut rest = xp;
    loop {
        let need = level_need(level);
        if rest < need {
            return (level, rest, need);
        }
        rest -= need;
        level += 1;
    }
}

pub fn date_string(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn week_of(day: i64) -> i64 {
    (day + 3).div_euclid(7)
}

fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn seed_of(user: &str) -> u64 {
    user.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn round_to(x: f64, step: u64) -> u64 {
    ((x / step as f64).round() as u64) * step
}

fn gen_quests(seed: u64, day: i64, recent: &VecDeque<DayStats>) -> Vec<Quest> {
    let n = recent.len().max(1) as f64;
    let avg = |f: fn(&DayStats) -> f64| recent.iter().map(f).sum::<f64>() / n;
    let reviews = avg(|s| s.reviews as f64).max(20.0);
    let minutes = avg(|s| s.time_ms as f64 / 60_000.0).max(5.0);
    let mature = avg(|s| s.mature as f64);
    let new_cards = avg(|s| s.new_cards as f64);

    let mut rng = mix(seed ^ day as u64);
    let mut next = move || {
        rng = mix(rng);
        rng
    };
    let factor = [0.8, 1.0, 1.2][(next() % 3) as usize];
    let review_target = round_to(reviews * factor, 5).max(10);

    let mut pool = vec![
        Quest {
            kind: QuestKind::Minutes,
            target: (minutes * factor).round().max(5.0) as u64,
        },
        Quest {
            kind: QuestKind::Combo,
            target: [20, 30, 50][(next() % 3) as usize].min(review_target),
        },
        Quest {
            kind: QuestKind::Sessions,
            target: 2,
        },
        Quest {
            kind: QuestKind::BeforeNoon,
            target: round_to(reviews * 0.3, 5).max(5),
        },
    ];
    if mature >= 5.0 {
        pool.push(Quest {
            kind: QuestKind::Mature,
            target: round_to(mature * 0.8, 5).max(5),
        });
    }
    if new_cards >= 1.0 {
        pool.push(Quest {
            kind: QuestKind::NewCards,
            target: (new_cards.round() as u64).max(3),
        });
    }

    let mut quests = vec![Quest {
        kind: QuestKind::Reviews,
        target: review_target,
    }];
    for _ in 0..2 {
        let i = (next() % pool.len() as u64) as usize;
        quests.push(pool.swap_remove(i));
    }
    quests
}

fn review_base_xp(r: &Review) -> f64 {
    if r.time_ms < 500 {
        return 1.0;
    }
    let base = match r.kind {
        1 => 10.0,
        0 | 2 => 6.0,
        _ => 2.0,
    };
    if r.last_ivl >= MATURE_IVL {
        base + 5.0
    } else {
        base
    }
}

fn collect_days(reviews: &[Review], clock: &Clock) -> (BTreeMap<i64, DayStats>, u64) {
    let mut days: BTreeMap<i64, DayStats> = BTreeMap::new();
    let mut seen = HashSet::new();
    let mut combo = 0u64;
    let mut prev: Option<(i64, i64)> = None;
    for r in reviews {
        let day = clock.day(r.id);
        let hour = clock.hour(r.id);
        let s = days.entry(day).or_default();
        let continues = prev.is_some_and(|(d, id)| d == day && r.id - id < SESSION_GAP_MS);
        if continues {
            combo += 1;
        } else {
            combo = 1;
            s.sessions += 1;
        }
        prev = Some((day, r.id));
        s.reviews += 1;
        s.time_ms += r.time_ms;
        s.max_combo = s.max_combo.max(combo);
        if seen.insert(r.cid) && r.kind == 0 {
            s.new_cards += 1;
        }
        if r.last_ivl >= MATURE_IVL {
            s.mature += 1;
        }
        let morning = hour >= clock.rollover_hour;
        if morning && hour < 12 {
            s.before_noon += 1;
        }
        if morning && hour < 7 {
            s.early = true;
        }
        if hour >= 23 || !morning {
            s.late = true;
        }
        s.review_xp += review_base_xp(r) * (1.0 + combo.min(100) as f64 / 200.0);
    }
    (days, combo)
}

pub fn compute(
    user: &str,
    display: &str,
    reviews: &[Review],
    clock: &Clock,
    now_ms: i64,
) -> Profile {
    let (days, last_combo) = collect_days(reviews, clock);
    let today = clock.day(now_ms);
    let seed = seed_of(user);
    let first = days.keys().next().copied().unwrap_or(today);

    let mut totals = Totals::default();
    let mut streak = 0u64;
    let mut freezes = 0u32;
    let mut frozen = BTreeSet::new();
    let mut recent: VecDeque<DayStats> = VecDeque::new();
    let mut unlocked: Vec<Option<i64>> = vec![None; ACHIEVEMENTS.len()];
    let mut day_xp: BTreeMap<i64, u64> = BTreeMap::new();
    let mut xp_total = 0u64;
    let mut today_quests = Vec::new();
    let mut events = Vec::new();
    let empty = DayStats::default();

    for day in first..=today {
        let stats = days.get(&day);
        if stats.is_some() {
            streak += 1;
            totals.best_streak = totals.best_streak.max(streak);
            if streak.is_multiple_of(u64::from(FREEZE_EVERY)) && freezes < MAX_FREEZES {
                freezes += 1;
            }
        } else if day != today {
            if streak > 0 && freezes > 0 {
                freezes -= 1;
                frozen.insert(day);
            } else {
                streak = 0;
            }
        }

        let quests = gen_quests(seed, day, &recent);
        let mut xp = 0u64;
        if let Some(s) = stats {
            xp += (s.review_xp * (1.0 + streak.min(30) as f64 / 100.0)).round() as u64;
            let done = quests.iter().filter(|q| q.progress(s) >= q.target).count() as u64;
            xp += done * QUEST_XP;
            totals.quests += done;
            if done == quests.len() as u64 {
                xp += ALL_QUESTS_XP;
                totals.perfect_days += 1;
            }
            totals.reviews += s.reviews;
            totals.time_ms += s.time_ms;
            totals.mature += s.mature;
            totals.new_cards += s.new_cards;
            totals.best_day = totals.best_day.max(s.reviews);
            totals.best_combo = totals.best_combo.max(s.max_combo);
            totals.early_days += u64::from(s.early);
            totals.late_days += u64::from(s.late);
            totals.days_active += 1;
            recent.push_back(s.clone());
            if recent.len() > RECENT_DAYS {
                recent.pop_front();
            }
        }
        for (i, d) in ACHIEVEMENTS.iter().enumerate() {
            if unlocked[i].is_none() && totals.value(d.metric) >= d.threshold {
                unlocked[i] = Some(day);
                xp += achievement_xp(d.metric, d.threshold);
            }
        }
        if day == today {
            let s = stats.unwrap_or(&empty);
            for (i, q) in quests.iter().enumerate() {
                let progress = q.progress(s);
                let view = QuestView {
                    title: q.title(),
                    progress: progress.min(q.target),
                    target: q.target,
                    done: progress >= q.target,
                    reward: QUEST_XP,
                };
                if view.done {
                    events.push(Event {
                        key: format!("quest:{day}:{i}"),
                        title: "Quest complete".into(),
                        body: format!("{} (+{QUEST_XP} XP)", view.title),
                    });
                }
                today_quests.push(view);
            }
        }
        xp_total += xp;
        if xp > 0 {
            day_xp.insert(day, xp);
        }
    }

    let (level, xp_into_level, xp_for_next) = level_for(xp_total);
    if level > 1 {
        events.push(Event {
            key: format!("level:{level}"),
            title: format!("Level {level}"),
            body: format!("You reached level {level} with {xp_total} XP."),
        });
    }

    let achievements = ACHIEVEMENTS
        .iter()
        .zip(&unlocked)
        .map(|(d, at)| {
            let id = format!("{:?}-{}", d.metric, d.threshold).to_lowercase();
            let reward = achievement_xp(d.metric, d.threshold);
            if at.is_some() {
                events.push(Event {
                    key: format!("achievement:{id}"),
                    title: format!("Achievement: {}", d.title),
                    body: format!("{} (+{reward} XP)", describe(d.metric, d.threshold)),
                });
            }
            AchievementView {
                id,
                title: d.title,
                description: describe(d.metric, d.threshold),
                reward,
                progress: totals.value(d.metric).min(d.threshold),
                target: d.threshold,
                unlocked: at.map(date_string),
            }
        })
        .collect();

    let heatmap = (today - HEATMAP_DAYS + 1..=today)
        .map(|day| HeatCell {
            date: date_string(day),
            reviews: days.get(&day).map_or(0, |s| s.reviews),
            xp: day_xp.get(&day).copied().unwrap_or(0),
            frozen: frozen.contains(&day),
        })
        .collect();

    let week_xp = day_xp
        .range(today - 6..=today)
        .filter(|(d, _)| week_of(**d) == week_of(today))
        .map(|(_, xp)| xp)
        .sum();

    let now = days.get(&today).unwrap_or(&empty);
    Profile {
        user: user.into(),
        display: display.into(),
        level,
        xp_total,
        xp_into_level,
        xp_for_next,
        week_xp,
        streak,
        freezes,
        at_risk: streak > 0 && !days.contains_key(&today),
        local_hour: clock.hour(now_ms),
        today: Today {
            reviews: now.reviews,
            minutes: (now.time_ms / 60_000) as u64,
            xp: day_xp.get(&today).copied().unwrap_or(0),
            max_combo: now.max_combo,
            current_combo: if reviews.last().is_some_and(|r| clock.day(r.id) == today) {
                last_combo
            } else {
                0
            },
            new_cards: now.new_cards,
        },
        lifetime: Lifetime {
            reviews: totals.reviews,
            hours: (totals.time_ms / 3_600_000) as u64,
            days_active: totals.days_active,
            best_streak: totals.best_streak,
            best_day: totals.best_day,
            best_combo: totals.best_combo,
            quests: totals.quests,
        },
        quests: today_quests,
        achievements,
        heatmap,
        last_review_id: reviews.last().map_or(0, |r| r.id),
        day: today,
        events,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 86_400_000;
    const NOON: i64 = 12 * 3_600_000;

    fn utc() -> Clock {
        Clock::default()
    }

    fn reviews_on(day: i64, count: i64, cid_base: i64) -> Vec<Review> {
        (0..count)
            .map(|i| Review {
                id: day * DAY_MS + NOON + i * 10_000,
                cid: cid_base + i,
                last_ivl: 0,
                time_ms: 5_000,
                kind: 1,
            })
            .collect()
    }

    fn history(days: &[i64]) -> Vec<Review> {
        days.iter()
            .flat_map(|d| reviews_on(*d, 5, d * 1000))
            .collect()
    }

    fn at(day: i64) -> i64 {
        day * DAY_MS + NOON + 3_600_000
    }

    #[test]
    fn day_respects_rollover_and_offset() {
        let clock = Clock {
            offset_west_min: -120,
            rollover_hour: 4,
        };
        let one_am_utc = 100 * DAY_MS + 3_600_000;
        assert_eq!(clock.hour(one_am_utc), 3);
        assert_eq!(clock.day(one_am_utc), 99);
        assert_eq!(clock.day(one_am_utc + 2 * 3_600_000), 100);
    }

    #[test]
    fn dates() {
        assert_eq!(date_string(0), "1970-01-01");
        assert_eq!(date_string(20_714), "2026-09-18");
        assert_eq!(date_string(19_782), "2024-02-29");
    }

    #[test]
    fn levels_are_monotonic() {
        assert_eq!(level_for(0), (1, 0, 100));
        assert_eq!(level_for(99).0, 1);
        assert_eq!(level_for(100), (2, 0, 280));
        let mut last = 0;
        for level in 1..200 {
            assert!(level_need(level) > last);
            last = level_need(level);
        }
    }

    #[test]
    fn streak_counts_consecutive_days() {
        let p = compute("a", "a", &history(&[10, 11, 12]), &utc(), at(12));
        assert_eq!(p.streak, 3);
        assert!(!p.at_risk);
    }

    #[test]
    fn today_without_reviews_is_at_risk_not_broken() {
        let p = compute("a", "a", &history(&[10, 11, 12]), &utc(), at(13));
        assert_eq!(p.streak, 3);
        assert!(p.at_risk);
    }

    #[test]
    fn missed_day_without_freeze_resets() {
        let p = compute("a", "a", &history(&[10, 11, 13]), &utc(), at(13));
        assert_eq!(p.streak, 1);
        assert_eq!(p.lifetime.best_streak, 2);
    }

    #[test]
    fn freeze_is_earned_and_spent() {
        let mut days: Vec<i64> = (1..=7).collect();
        days.push(9);
        let p = compute("a", "a", &history(&days), &utc(), at(9));
        assert_eq!(p.streak, 8);
        assert_eq!(p.freezes, 0);
        assert!(
            p.heatmap
                .iter()
                .any(|c| c.frozen && c.date == date_string(8))
        );
    }

    #[test]
    fn combo_breaks_on_gap() {
        let mut reviews = reviews_on(5, 10, 0);
        let mut later = reviews_on(5, 4, 100);
        for r in &mut later {
            r.id += 3_600_000;
        }
        reviews.extend(later);
        let p = compute("a", "a", &reviews, &utc(), at(5));
        assert_eq!(p.today.max_combo, 10);
        assert_eq!(p.today.current_combo, 4);
        assert_eq!(p.today.reviews, 14);
    }

    #[test]
    fn answer_button_never_matters() {
        let r = Review {
            id: 0,
            cid: 1,
            last_ivl: 30,
            time_ms: 4_000,
            kind: 1,
        };
        assert_eq!(review_base_xp(&r), 15.0);
    }

    #[test]
    fn quests_are_deterministic_and_distinct() {
        let reviews = history(&[1, 2, 3]);
        let a = compute("a", "a", &reviews, &utc(), at(3));
        let b = compute("a", "a", &reviews, &utc(), at(3));
        assert_eq!(a.quests.len(), 3);
        let titles = |p: &Profile| p.quests.iter().map(|q| q.title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(&a), titles(&b));
        let unique: HashSet<_> = titles(&a).into_iter().collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn xp_sums_into_heatmap_and_total() {
        let p = compute("a", "a", &history(&[1, 2, 3]), &utc(), at(3));
        let heat: u64 = p.heatmap.iter().map(|c| c.xp).sum();
        assert_eq!(heat, p.xp_total);
        assert!(p.xp_total > 0);
        assert_eq!(p.week_xp, p.xp_total);
    }

    #[test]
    fn achievements_unlock_with_events() {
        let reviews: Vec<Review> = reviews_on(1, 120, 0);
        let p = compute("a", "a", &reviews, &utc(), at(1));
        let first = p
            .achievements
            .iter()
            .find(|a| a.id == "reviews-100")
            .unwrap();
        assert_eq!(first.unlocked.as_deref(), Some("1970-01-02"));
        assert!(p.events.iter().any(|e| e.key == "achievement:reviews-100"));
        assert!(
            p.events
                .iter()
                .any(|e| e.key == "achievement:dayreviews-100")
        );
    }

    #[test]
    fn empty_history() {
        let p = compute("a", "a", &[], &utc(), at(50));
        assert_eq!(p.level, 1);
        assert_eq!(p.streak, 0);
        assert_eq!(p.quests.len(), 3);
        assert_eq!(p.heatmap.len(), HEATMAP_DAYS as usize);
    }
}
