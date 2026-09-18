use chrono::{DateTime, Datelike, Duration, NaiveDateTime, TimeZone};
use chrono_tz::Tz;
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

    fn day_start_ms(&self, day: i64) -> i64 {
        (day * 86400 + self.rollover_hour * 3600 + self.offset_west_min * 60) * 1000
    }
}

/// The leaderboard week shared by every player: Monday to Sunday in one time zone,
/// turning over at the same hour for everyone.
#[derive(Clone, Copy, Debug)]
pub struct Week {
    pub tz: Tz,
    pub rollover_hour: u32,
}

impl Default for Week {
    fn default() -> Self {
        Self {
            tz: Tz::UTC,
            rollover_hour: 4,
        }
    }
}

impl Week {
    fn shifted(&self, ms: i64) -> NaiveDateTime {
        DateTime::from_timestamp_millis(ms)
            .unwrap_or_default()
            .with_timezone(&self.tz)
            .naive_local()
            - Duration::hours(i64::from(self.rollover_hour))
    }

    fn of(&self, ms: i64) -> (i32, u32) {
        let week = self.shifted(ms).date().iso_week();
        (week.year(), week.week())
    }

    /// When the week containing `ms` ends, in milliseconds since the epoch.
    pub fn end_after(&self, ms: i64) -> i64 {
        let date = self.shifted(ms).date();
        let monday = date + Duration::days(7 - i64::from(date.weekday().num_days_from_monday()));
        let boundary = monday
            .and_hms_opt(self.rollover_hour, 0, 0)
            .unwrap_or_default();
        self.tz
            .from_local_datetime(&boundary)
            .earliest()
            .map_or(ms + 7 * 86_400_000, |t| t.timestamp_millis())
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
    relearns: u64,
    max_ivl: i64,
    first_seen: u64,
    longest_session_ms: i64,
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
    DaysActive,
    Relearns,
    MaxInterval,
    DistinctCards,
    LongestSession,
    MaxSessions,
    BestMonth,
    WeekendDays,
    Comebacks,
    FreezesUsed,
    FastDays,
    NewYearDays,
    Anniversaries,
    Years,
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
    def(Metric::Reviews, 250_000, "Quarter Million"),
    def(Metric::Reviews, 500_000, "Half a Million"),
    def(Metric::Reviews, 1_000_000, "The Millionaire"),
    def(Metric::Streak, 3, "Warming Up"),
    def(Metric::Streak, 7, "Full Week"),
    def(Metric::Streak, 14, "Fortnight"),
    def(Metric::Streak, 30, "Month of Mondays"),
    def(Metric::Streak, 60, "Habit Formed"),
    def(Metric::Streak, 100, "Centurion"),
    def(Metric::Streak, 200, "Unbreakable"),
    def(Metric::Streak, 365, "Orbit Complete"),
    def(Metric::Streak, 500, "Five Hundred Sunrises"),
    def(Metric::Streak, 730, "Two Orbits"),
    def(Metric::Streak, 1_000, "Thousand Days"),
    def(Metric::Streak, 1_500, "Unstoppable"),
    def(Metric::Streak, 2_000, "Perennial"),
    def(Metric::DayReviews, 100, "Busy Day"),
    def(Metric::DayReviews, 250, "Marathon"),
    def(Metric::DayReviews, 500, "Ultramarathon"),
    def(Metric::DayReviews, 750, "Iron Lung"),
    def(Metric::DayReviews, 1_000, "Thousand in a Day"),
    def(Metric::DayReviews, 1_500, "Grinder"),
    def(Metric::DayReviews, 2_000, "Madness"),
    def(Metric::Combo, 50, "In the Zone"),
    def(Metric::Combo, 100, "Flow State"),
    def(Metric::Combo, 250, "Trance"),
    def(Metric::Combo, 500, "Zen"),
    def(Metric::Combo, 1_000, "Nirvana"),
    def(Metric::Hours, 10, "Ten Hours In"),
    def(Metric::Hours, 50, "Fifty Hours In"),
    def(Metric::Hours, 100, "Hundred Hours In"),
    def(Metric::Hours, 500, "Scholar"),
    def(Metric::Hours, 1_000, "Thousand Hours"),
    def(Metric::Hours, 2_000, "Two Thousand Hours"),
    def(Metric::Hours, 5_000, "Lifer"),
    def(Metric::Mature, 100, "Long Term"),
    def(Metric::Mature, 1_000, "Deep Roots"),
    def(Metric::Mature, 10_000, "Old Growth"),
    def(Metric::Mature, 25_000, "Ancient Forest"),
    def(Metric::Mature, 50_000, "Redwood"),
    def(Metric::Mature, 100_000, "Bedrock"),
    def(Metric::NewCards, 100, "Collector"),
    def(Metric::NewCards, 1_000, "Curator"),
    def(Metric::NewCards, 5_000, "Archivist"),
    def(Metric::NewCards, 10_000, "Encyclopedist"),
    def(Metric::NewCards, 25_000, "Lexicographer"),
    def(Metric::NewCards, 50_000, "Polymath"),
    def(Metric::Quests, 10, "Adventurer"),
    def(Metric::Quests, 50, "Quest Hound"),
    def(Metric::Quests, 200, "Completionist"),
    def(Metric::Quests, 500, "Quest Master"),
    def(Metric::Quests, 1_000, "Legend"),
    def(Metric::Quests, 2_500, "Mythic"),
    def(Metric::PerfectDays, 7, "Perfect Week"),
    def(Metric::PerfectDays, 30, "Perfect Month"),
    def(Metric::PerfectDays, 100, "Perfectionist"),
    def(Metric::PerfectDays, 365, "Flawless Year"),
    def(Metric::EarlyDays, 5, "Early Bird"),
    def(Metric::EarlyDays, 30, "Dawn Patrol"),
    def(Metric::EarlyDays, 100, "Sunrise Scholar"),
    def(Metric::LateDays, 5, "Night Owl"),
    def(Metric::LateDays, 30, "Nocturnal"),
    def(Metric::LateDays, 100, "Creature of the Night"),
    def(Metric::DaysActive, 30, "Regular"),
    def(Metric::DaysActive, 100, "Hundred Days"),
    def(Metric::DaysActive, 365, "A Year of Days"),
    def(Metric::DaysActive, 730, "Two Years of Days"),
    def(Metric::DaysActive, 1_000, "Thousand Days Studied"),
    def(Metric::DaysActive, 2_000, "Two Thousand Days Studied"),
    def(Metric::Relearns, 100, "Second Chance"),
    def(Metric::Relearns, 1_000, "Persistence"),
    def(Metric::Relearns, 10_000, "Never Give Up"),
    def(Metric::Relearns, 50_000, "Sisyphus"),
    def(Metric::MaxInterval, 180, "Half-Year Memory"),
    def(Metric::MaxInterval, 365, "Year-Old Memory"),
    def(Metric::MaxInterval, 1_825, "Five-Year Memory"),
    def(Metric::MaxInterval, 3_650, "Decade Memory"),
    def(Metric::DistinctCards, 1_000, "Wide Net"),
    def(Metric::DistinctCards, 5_000, "Big Deck"),
    def(Metric::DistinctCards, 20_000, "Vast Library"),
    def(Metric::DistinctCards, 50_000, "Everything Everywhere"),
    def(Metric::LongestSession, 60, "Deep Work"),
    def(Metric::LongestSession, 120, "Two-Hour Sitting"),
    def(Metric::LongestSession, 240, "Iron Chair"),
    def(Metric::MaxSessions, 5, "Snacker"),
    def(Metric::MaxSessions, 10, "Grazer"),
    def(Metric::MaxSessions, 20, "Can't Stop"),
    def(Metric::BestMonth, 1_000, "Busy Month"),
    def(Metric::BestMonth, 3_000, "Big Month"),
    def(Metric::BestMonth, 10_000, "Monster Month"),
    def(Metric::BestMonth, 20_000, "Month of Madness"),
    def(Metric::WeekendDays, 10, "Weekend Warrior"),
    def(Metric::WeekendDays, 52, "Weekend Regular"),
    def(Metric::WeekendDays, 200, "No Days Off"),
    def(Metric::Comebacks, 1, "Comeback"),
    def(Metric::Comebacks, 3, "Phoenix"),
    def(Metric::FreezesUsed, 1, "Saved by the Ice"),
    def(Metric::FreezesUsed, 10, "Glacier"),
    def(Metric::FreezesUsed, 50, "Ice Age"),
    def(Metric::FastDays, 1, "Speed Demon"),
    def(Metric::FastDays, 10, "Lightning"),
    def(Metric::FastDays, 50, "Quicksilver"),
    def(Metric::NewYearDays, 1, "New Year, Same Me"),
    def(Metric::NewYearDays, 3, "Tradition"),
    def(Metric::Anniversaries, 1, "Anniversary"),
    def(Metric::Anniversaries, 5, "Old Friends"),
    def(Metric::Anniversaries, 10, "Lifelong"),
    def(Metric::Years, 1, "One Year In"),
    def(Metric::Years, 3, "Three Years In"),
    def(Metric::Years, 5, "Veteran"),
    def(Metric::Years, 10, "Decade of Anki"),
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
    relearns: u64,
    max_ivl: u64,
    distinct_cards: u64,
    longest_session_ms: i64,
    max_sessions: u64,
    best_month: u64,
    weekend_days: u64,
    comebacks: u64,
    freezes_used: u64,
    fast_days: u64,
    new_year_days: u64,
    anniversaries: u64,
    years: u64,
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
            Metric::DaysActive => self.days_active,
            Metric::Relearns => self.relearns,
            Metric::MaxInterval => self.max_ivl,
            Metric::DistinctCards => self.distinct_cards,
            Metric::LongestSession => (self.longest_session_ms / 60_000) as u64,
            Metric::MaxSessions => self.max_sessions,
            Metric::BestMonth => self.best_month,
            Metric::WeekendDays => self.weekend_days,
            Metric::Comebacks => self.comebacks,
            Metric::FreezesUsed => self.freezes_used,
            Metric::FastDays => self.fast_days,
            Metric::NewYearDays => self.new_year_days,
            Metric::Anniversaries => self.anniversaries,
            Metric::Years => self.years,
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
        Metric::DaysActive => format!("Study on {n} different days"),
        Metric::Relearns => format!("Relearn {n} forgotten cards"),
        Metric::MaxInterval => format!("Recall a card last seen {n} days before"),
        Metric::DistinctCards => format!("Review {n} different cards"),
        Metric::LongestSession => format!("Study {n} minutes in one sitting"),
        Metric::MaxSessions => format!("Study in {n} separate sessions in one day"),
        Metric::BestMonth => format!("Review {n} cards in one calendar month"),
        Metric::WeekendDays => format!("Study on {n} Saturdays or Sundays"),
        Metric::Comebacks => match n {
            1 => "Come back after a month away".into(),
            _ => format!("Come back after a month away {n} times"),
        },
        Metric::FreezesUsed => format!("Have a streak freeze save you {n} times"),
        Metric::FastDays => {
            format!("Review 100+ cards in a day averaging under 5 seconds, {n} times")
        }
        Metric::NewYearDays => format!("Study on New Year's Day {n} times"),
        Metric::Anniversaries => format!("Study on the anniversary of your first review {n} times"),
        Metric::Years => format!("Keep going for {n} years since your first review"),
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
    let (y, m, d) = civil(day);
    format!("{y:04}-{m:02}-{d:02}")
}

fn civil(day: i64) -> (i64, i64, i64) {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

fn is_weekend(day: i64) -> bool {
    (day + 3).rem_euclid(7) >= 5
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
    let mut session_ms = 0i64;
    let mut prev: Option<(i64, i64)> = None;
    for r in reviews {
        let day = clock.day(r.id);
        let hour = clock.hour(r.id);
        let s = days.entry(day).or_default();
        let continues = prev.is_some_and(|(d, id)| d == day && r.id - id < SESSION_GAP_MS);
        if continues {
            combo += 1;
            session_ms += r.time_ms;
        } else {
            combo = 1;
            session_ms = r.time_ms;
            s.sessions += 1;
        }
        prev = Some((day, r.id));
        s.reviews += 1;
        s.time_ms += r.time_ms;
        s.max_combo = s.max_combo.max(combo);
        s.longest_session_ms = s.longest_session_ms.max(session_ms);
        s.max_ivl = s.max_ivl.max(r.last_ivl);
        if r.kind == 2 {
            s.relearns += 1;
        }
        if seen.insert(r.cid) {
            s.first_seen += 1;
            if r.kind == 0 {
                s.new_cards += 1;
            }
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
    week: &Week,
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
    let (first_year, first_month, first_date) = civil(first);
    let mut month = (first_year, first_month);
    let mut month_reviews = 0u64;
    let mut last_active: Option<i64> = None;

    for day in first..=today {
        let stats = days.get(&day);
        let (year, month_of_year, date) = civil(day);
        if (year, month_of_year) != month {
            month = (year, month_of_year);
            month_reviews = 0;
        }
        totals.years = ((day - first) / 365) as u64;
        if let Some(s) = stats {
            month_reviews += s.reviews;
            totals.best_month = totals.best_month.max(month_reviews);
            totals.relearns += s.relearns;
            totals.max_ivl = totals.max_ivl.max(s.max_ivl.max(0) as u64);
            totals.distinct_cards += s.first_seen;
            totals.longest_session_ms = totals.longest_session_ms.max(s.longest_session_ms);
            totals.max_sessions = totals.max_sessions.max(s.sessions);
            totals.weekend_days += u64::from(is_weekend(day));
            totals.fast_days += u64::from(s.reviews >= 100 && s.time_ms < s.reviews as i64 * 5_000);
            totals.new_year_days += u64::from(month_of_year == 1 && date == 1);
            totals.anniversaries +=
                u64::from(year > first_year && month_of_year == first_month && date == first_date);
            if last_active.is_some_and(|last| day - last >= 30) {
                totals.comebacks += 1;
            }
            last_active = Some(day);
        }
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
                totals.freezes_used += 1;
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

    let this_week = week.of(now_ms);
    let week_xp = day_xp
        .range(today - 9..=today)
        .filter(|(d, _)| week.of(clock.day_start_ms(**d)) == this_week)
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
    fn week_is_shared_across_time_zones() {
        let berlin = chrono_tz::Europe::Berlin;
        let week = Week {
            tz: berlin,
            rollover_hour: 4,
        };
        let at_berlin = |d, h| {
            berlin
                .with_ymd_and_hms(2026, 9, d, h, 0, 0)
                .unwrap()
                .timestamp_millis()
        };
        let buenos_aires = Clock {
            offset_west_min: 180,
            rollover_hour: 4,
        };
        let review = |id| Review {
            id,
            cid: id,
            last_ivl: 0,
            time_ms: 5_000,
            kind: 1,
        };

        assert_eq!(week.of(at_berlin(21, 3)), week.of(at_berlin(20, 12)));
        assert_ne!(week.of(at_berlin(21, 5)), week.of(at_berlin(20, 12)));
        assert_eq!(week.end_after(at_berlin(21, 10)), at_berlin(28, 4));
        assert_eq!(week.end_after(at_berlin(21, 3)), at_berlin(21, 4));

        let sunday = at_berlin(20, 20);
        let late_sunday = at_berlin(21, 8);
        let p = compute(
            "a",
            "a",
            &[review(sunday), review(late_sunday)],
            &buenos_aires,
            &week,
            late_sunday,
        );
        assert_eq!(
            p.week_xp, 0,
            "their Sunday started before the shared week did"
        );
        assert_eq!(p.today.reviews, 2);

        let monday = at_berlin(21, 10);
        let p = compute(
            "a",
            "a",
            &[review(sunday), review(monday)],
            &buenos_aires,
            &week,
            monday,
        );
        assert!(
            p.week_xp > 0,
            "their Monday started after the shared week did"
        );
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
        let p = compute(
            "a",
            "a",
            &history(&[10, 11, 12]),
            &utc(),
            &Week::default(),
            at(12),
        );
        assert_eq!(p.streak, 3);
        assert!(!p.at_risk);
    }

    #[test]
    fn today_without_reviews_is_at_risk_not_broken() {
        let p = compute(
            "a",
            "a",
            &history(&[10, 11, 12]),
            &utc(),
            &Week::default(),
            at(13),
        );
        assert_eq!(p.streak, 3);
        assert!(p.at_risk);
    }

    #[test]
    fn missed_day_without_freeze_resets() {
        let p = compute(
            "a",
            "a",
            &history(&[10, 11, 13]),
            &utc(),
            &Week::default(),
            at(13),
        );
        assert_eq!(p.streak, 1);
        assert_eq!(p.lifetime.best_streak, 2);
    }

    #[test]
    fn freeze_is_earned_and_spent() {
        let mut days: Vec<i64> = (1..=7).collect();
        days.push(9);
        let p = compute("a", "a", &history(&days), &utc(), &Week::default(), at(9));
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
        let p = compute("a", "a", &reviews, &utc(), &Week::default(), at(5));
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
        let a = compute("a", "a", &reviews, &utc(), &Week::default(), at(3));
        let b = compute("a", "a", &reviews, &utc(), &Week::default(), at(3));
        assert_eq!(a.quests.len(), 3);
        let titles = |p: &Profile| p.quests.iter().map(|q| q.title.clone()).collect::<Vec<_>>();
        assert_eq!(titles(&a), titles(&b));
        let unique: HashSet<_> = titles(&a).into_iter().collect();
        assert_eq!(unique.len(), 3);
    }

    #[test]
    fn xp_sums_into_heatmap_and_total() {
        let p = compute(
            "a",
            "a",
            &history(&[1, 2, 3]),
            &utc(),
            &Week::default(),
            at(3),
        );
        let heat: u64 = p.heatmap.iter().map(|c| c.xp).sum();
        assert_eq!(heat, p.xp_total);
        assert!(p.xp_total > 0);
        assert_eq!(p.week_xp, p.xp_total);
    }

    #[test]
    fn achievements_unlock_with_events() {
        let reviews: Vec<Review> = reviews_on(1, 120, 0);
        let p = compute("a", "a", &reviews, &utc(), &Week::default(), at(1));
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
    fn calendar_and_memory_achievements() {
        let mut reviews = reviews_on(0, 5, 0);
        reviews.extend(reviews_on(2, 5, 100));
        reviews.extend(reviews_on(40, 5, 200));
        let mut fast = reviews_on(365, 120, 1000);
        for r in &mut fast {
            r.time_ms = 3_000;
        }
        fast[0].last_ivl = 400;
        reviews.extend(fast);
        let p = compute("a", "a", &reviews, &utc(), &Week::default(), at(365));
        let unlocked = |id: &str| {
            p.achievements
                .iter()
                .find(|a| a.id == id)
                .unwrap_or_else(|| panic!("no achievement {id}"))
                .unlocked
                .is_some()
        };
        for id in [
            "comebacks-1",
            "anniversaries-1",
            "years-1",
            "newyeardays-1",
            "maxinterval-365",
            "fastdays-1",
            "dayreviews-100",
        ] {
            assert!(unlocked(id), "{id} should be unlocked");
        }
        assert!(!unlocked("comebacks-3"));
        assert!(!unlocked("maxinterval-1825"));
        let weekend = p
            .achievements
            .iter()
            .find(|a| a.id == "weekenddays-10")
            .unwrap();
        assert_eq!(weekend.progress, 1);
    }

    #[test]
    fn achievement_ids_are_unique() {
        let p = compute("a", "a", &[], &utc(), &Week::default(), at(1));
        let ids: HashSet<_> = p.achievements.iter().map(|a| a.id.clone()).collect();
        assert_eq!(ids.len(), p.achievements.len());
    }

    #[test]
    fn empty_history() {
        let p = compute("a", "a", &[], &utc(), &Week::default(), at(50));
        assert_eq!(p.level, 1);
        assert_eq!(p.streak, 0);
        assert_eq!(p.quests.len(), 3);
        assert_eq!(p.heatmap.len(), HEATMAP_DAYS as usize);
    }
}
