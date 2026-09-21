mod decks;
mod game;
mod store;

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use game::{Clock, Periods, Profile, Records, Review, Week};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use store::{Error, Store};

const POLL: Duration = Duration::from_secs(20);
const MAX_PENDING: usize = 5000;
const PUSH_BUDGET: Duration = Duration::from_secs(20);
/// How many people a record names: the holder and whoever came closest.
const PODIUM: usize = 3;

#[derive(Deserialize, Clone, Default)]
struct UserConfig {
    display: Option<String>,
    ntfy_topic: Option<String>,
    token: Option<String>,
    token_file: Option<PathBuf>,
}

#[derive(Deserialize, Clone)]
struct Config {
    #[serde(default = "default_addr")]
    addr: String,
    sync_base: Option<PathBuf>,
    #[serde(default = "default_state")]
    state_dir: PathBuf,
    ntfy: Option<String>,
    #[serde(default = "default_remind_hour")]
    remind_hour: i64,
    public_url: Option<String>,
    #[serde(default = "default_week_timezone")]
    week_timezone: String,
    #[serde(default = "default_week_rollover_hour")]
    week_rollover_hour: u32,
    #[serde(default)]
    users: HashMap<String, UserConfig>,
}

fn default_week_timezone() -> String {
    "UTC".into()
}

fn default_week_rollover_hour() -> u32 {
    4
}

fn default_addr() -> String {
    "127.0.0.1:8097".into()
}

fn default_state() -> PathBuf {
    "state".into()
}

fn default_remind_hour() -> i64 {
    20
}

struct Player {
    reviews: Vec<Review>,
    clock: Clock,
}

struct App {
    config: Config,
    week: Week,
    store: Mutex<Store>,
    players: RwLock<HashMap<String, Player>>,
}

impl App {
    fn display(&self, user: &str) -> String {
        self.config
            .users
            .get(user)
            .and_then(|u| u.display.clone())
            .unwrap_or_else(|| user.into())
    }

    fn profile(&self, user: &str) -> Option<Profile> {
        self.preview(user, &[])
    }

    fn preview(&self, user: &str, pending: &[Review]) -> Option<Profile> {
        let players = self.players.read().unwrap();
        let player = players.get(user)?;
        let last = player.reviews.last().map_or(0, |r| r.id);
        let mut fresh: Vec<Review> = pending.iter().filter(|r| r.id > last).copied().collect();
        fresh.sort_by_key(|r| r.id);
        fresh.dedup_by_key(|r| r.id);
        let merged = [player.reviews.as_slice(), &fresh].concat();
        Some(game::compute(
            user,
            &self.display(user),
            &merged,
            &player.clock,
            &self.week,
            now_ms(),
        ))
    }

    fn profiles(&self) -> Vec<Profile> {
        let users: Vec<String> = self.players.read().unwrap().keys().cloned().collect();
        users.iter().filter_map(|u| self.profile(u)).collect()
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[derive(Serialize)]
struct Standing {
    user: String,
    display: String,
    level: u64,
    xp_total: u64,
    week_xp: u64,
    xp: u64,
    period: String,
    periods: Periods,
    streak: u64,
    today_reviews: u64,
}

#[derive(Deserialize)]
struct BoardQuery {
    period: Option<String>,
}

async fn leaderboard(
    State(app): State<Arc<App>>,
    Query(query): Query<BoardQuery>,
) -> Json<Vec<Standing>> {
    let period = query
        .period
        .filter(|name| Periods::NAMES.contains(&name.as_str()))
        .unwrap_or_else(|| "week".into());
    let mut standings: Vec<Standing> = app
        .profiles()
        .into_iter()
        .map(|p| Standing {
            user: p.user,
            display: p.display,
            level: p.level,
            xp_total: p.xp_total,
            week_xp: p.week_xp,
            xp: p.periods.get(&period),
            period: period.clone(),
            periods: p.periods,
            streak: p.streak,
            today_reviews: p.today.reviews,
        })
        .collect();
    standings.sort_by_key(|s| std::cmp::Reverse((s.xp, s.xp_total)));
    Json(standings)
}

async fn profile(
    State(app): State<Arc<App>>,
    UrlPath(user): UrlPath<String>,
) -> Result<Json<Profile>, StatusCode> {
    app.profile(&user).map(Json).ok_or(StatusCode::NOT_FOUND)
}

#[derive(Deserialize)]
struct Pending {
    reviews: Vec<Review>,
}

async fn preview(
    State(app): State<Arc<App>>,
    UrlPath(user): UrlPath<String>,
    Json(pending): Json<Pending>,
) -> Result<Json<Profile>, StatusCode> {
    if pending.reviews.len() > MAX_PENDING {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    app.preview(&user, &pending.reviews)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

#[derive(Deserialize)]
struct Upload {
    reviews: Vec<Review>,
    #[serde(default)]
    deleted: Vec<i64>,
    clock: Clock,
    #[serde(default)]
    silent: bool,
    decks: Option<Vec<decks::Snapshot>>,
    #[serde(default)]
    catalog: bool,
}

fn authorized(app: &App, user: &str, headers: &HeaderMap) -> bool {
    let Some(expected) = app.config.users.get(user).and_then(|u| u.token.as_deref()) else {
        return false;
    };
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    !expected.is_empty()
        && given.len() == expected.len()
        && given
            .bytes()
            .zip(expected.bytes())
            .fold(0, |acc, (a, b)| acc | (a ^ b))
            == 0
}

async fn upload(
    State(app): State<Arc<App>>,
    UrlPath(user): UrlPath<String>,
    headers: HeaderMap,
    Json(upload): Json<Upload>,
) -> Result<Json<UploadResponse>, StatusCode> {
    if !authorized(&app, &user, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if upload.reviews.len() > MAX_PENDING || upload.deleted.len() > MAX_PENDING {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    if upload
        .decks
        .as_ref()
        .is_some_and(|decks| decks.len() > decks::MAX_DECKS)
    {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    if !decks::valid_clock(upload.clock)
        || upload
            .decks
            .as_ref()
            .is_some_and(|decks| !decks::valid_snapshots(decks))
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    let reviews: Vec<Review> = upload.reviews.into_iter().filter(|r| r.kind < 4).collect();
    let store_error = |e: Error| {
        eprintln!("upload for {user} failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    };
    let mut store = app.store.lock().unwrap();
    let silent = upload.silent || !store.is_known(&user).map_err(store_error)?;
    store
        .upsert(&user, &reviews, &upload.deleted, upload.clock)
        .map_err(store_error)?;
    let mut announced = Vec::new();
    if let Some(decks) = &upload.decks {
        announced = store
            .record_decks(
                &user,
                &app.display(&user),
                decks,
                upload.clock,
                silent,
                now_ms(),
            )
            .map_err(store_error)?;
        if upload.catalog {
            store.prune_decks(&user, decks).map_err(store_error)?;
        }
    }
    let player = load_player(&store, &user).map_err(store_error)?;
    app.players.write().unwrap().insert(user.clone(), player);
    let profile = app.profile(&user).ok_or(StatusCode::NOT_FOUND)?;
    if silent {
        for event in &profile.events {
            store.mark_seen(&user, &event.key).map_err(store_error)?;
        }
    }
    Ok(Json(UploadResponse { profile, announced }))
}

fn deck_settings(app: &App, store: &Store, user: &str) -> Result<decks::Settings, Error> {
    let mut users: BTreeSet<String> = app.config.users.keys().cloned().collect();
    let ntfy_enabled = app
        .config
        .ntfy
        .as_deref()
        .is_some_and(|base| !base.trim().is_empty());
    users.retain(|candidate| {
        candidate != user
            && app.config.users.get(candidate).is_some_and(|config| {
                config
                    .token
                    .as_deref()
                    .is_some_and(|token| !token.is_empty())
                    || (ntfy_enabled
                        && config
                            .ntfy_topic
                            .as_deref()
                            .is_some_and(|topic| !topic.trim().is_empty()))
            })
    });
    Ok(decks::Settings {
        decks: store.decks(user)?,
        recipients: users
            .into_iter()
            .map(|user| decks::Recipient {
                display: app.display(&user),
                user,
            })
            .collect(),
        nudges: store.nudges_enabled(user)?,
    })
}

async fn get_decks(
    State(app): State<Arc<App>>,
    UrlPath(user): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Json<decks::Settings>, StatusCode> {
    if !authorized(&app, &user, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    deck_settings(&app, &app.store.lock().unwrap(), &user)
        .map(Json)
        .map_err(|e| {
            eprintln!("deck settings for {user} failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

async fn set_decks(
    State(app): State<Arc<App>>,
    UrlPath(user): UrlPath<String>,
    headers: HeaderMap,
    Json(update): Json<decks::SettingsUpdate>,
) -> Result<Json<decks::Settings>, StatusCode> {
    if !authorized(&app, &user, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if update.decks.len() > decks::MAX_DECKS {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    let store_error = |e: Error| {
        eprintln!("deck settings for {user} failed: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    };
    let mut store = app.store.lock().unwrap();
    let current = deck_settings(&app, &store, &user).map_err(store_error)?;
    if !decks::valid_preferences(&update.decks, &current.decks, &current.recipients) {
        return Err(StatusCode::BAD_REQUEST);
    }
    store
        .set_deck_preferences(&user, &update.decks)
        .map_err(store_error)?;
    if let Some(nudges) = update.nudges {
        store.set_nudges(&user, nudges).map_err(store_error)?;
    }
    deck_settings(&app, &store, &user)
        .map(Json)
        .map_err(store_error)
}

async fn notifications(
    State(app): State<Arc<App>>,
    UrlPath(user): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<decks::Notification>>, StatusCode> {
    if !authorized(&app, &user, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    app.store
        .lock()
        .unwrap()
        .notifications(&user, now_ms())
        .map(Json)
        .map_err(|e| {
            eprintln!("notifications for {user} failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

#[derive(Debug, Serialize)]
struct RecordHolder {
    user: String,
    display: String,
    value: u64,
    detail: u64,
    at: i64,
}

#[derive(Debug, Serialize)]
struct RecordBoard {
    window: String,
    unit: &'static str,
    holders: Vec<RecordHolder>,
}

fn podium(
    window: &str,
    unit: &'static str,
    profiles: &[Profile],
    of: impl Fn(&Profile) -> (u64, u64, i64),
) -> RecordBoard {
    let mut holders: Vec<RecordHolder> = profiles
        .iter()
        .map(|player| (player, of(player)))
        .filter(|(_, (value, _, _))| *value > 0)
        .map(|(player, (value, detail, at))| RecordHolder {
            user: player.user.clone(),
            display: player.display.clone(),
            value,
            detail,
            at,
        })
        .collect();
    holders.sort_by_key(|holder| std::cmp::Reverse((holder.value, holder.detail)));
    holders.truncate(PODIUM);
    RecordBoard {
        window: window.to_string(),
        unit,
        holders,
    }
}

/// The best hour, day, week, month and year anyone here has ever had, plus the
/// longest streak and the most days studied. Each names whoever came closest,
/// so a near miss is visible rather than hidden.
async fn records(State(app): State<Arc<App>>) -> Json<Vec<RecordBoard>> {
    let profiles = app.profiles();
    let mut board: Vec<RecordBoard> = Records::NAMES
        .iter()
        .map(|window| {
            podium(window, "xp", &profiles, |player| {
                let record = player.records.get(window);
                (record.xp, record.reviews, record.at)
            })
        })
        .collect();
    board.push(podium("streak", "days", &profiles, |player| {
        (
            player.lifetime.best_streak,
            0,
            player.lifetime.best_streak_at,
        )
    }));
    board.push(podium("days", "days", &profiles, |player| {
        (player.lifetime.days_active, 0, player.lifetime.first_day_at)
    }));
    Json(board)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplyRequest {
    notification: i64,
    message: String,
}

#[derive(Debug, Serialize)]
struct ReplyResponse {
    sent_to: String,
}

/// Answers a deck completion, so finishing a deck starts a conversation.
async fn reply(
    State(app): State<Arc<App>>,
    UrlPath(user): UrlPath<String>,
    headers: HeaderMap,
    Json(request): Json<ReplyRequest>,
) -> Result<Json<ReplyResponse>, StatusCode> {
    if !authorized(&app, &user, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let message = request.message.trim();
    if !decks::valid_message(message) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let sender = app
        .store
        .lock()
        .unwrap()
        .reply(
            &user,
            &app.display(&user),
            request.notification,
            message,
            now_ms(),
        )
        .map_err(|e| {
            eprintln!("reply from {user} failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(ReplyResponse {
        sent_to: app.display(&sender),
    }))
}

/// The player's profile, plus any deck completion this upload just announced.
#[derive(Debug, Serialize)]
struct UploadResponse {
    #[serde(flatten)]
    profile: Profile,
    announced: Vec<decks::Announcement>,
}

#[derive(Serialize)]
struct WeekInfo {
    ends_at: i64,
    timezone: String,
}

async fn week_info(State(app): State<Arc<App>>) -> Json<WeekInfo> {
    Json(WeekInfo {
        ends_at: app.week.end_after(now_ms()),
        timezone: app.config.week_timezone.clone(),
    })
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

/// The dashboard is one page; `/day`, `/month` and the rest pick a leaderboard period.
async fn period_page(UrlPath(period): UrlPath<String>) -> Result<Html<&'static str>, StatusCode> {
    if Periods::NAMES.contains(&period.as_str()) {
        Ok(index().await)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn manifest() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/manifest+json")],
        include_str!("../static/manifest.webmanifest"),
    )
}

async fn icon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("../static/icon.svg"),
    )
}

fn push_destination<'a>(config: &'a Config, user: &str) -> Option<(&'a str, &'a str)> {
    let base = config.ntfy.as_deref()?.trim();
    let topic = config.users.get(user)?.ntfy_topic.as_deref()?.trim();
    (!base.is_empty() && !topic.is_empty()).then_some((base, topic))
}

fn push(
    config: &Config,
    user: &str,
    title: &str,
    body: &str,
    tag: &str,
    timeout: Duration,
) -> Result<(), Error> {
    let (base, topic) = push_destination(config, user).ok_or("no ntfy destination")?;
    let mut request = ureq::post(&format!("{}/{topic}", base.trim_end_matches('/')))
        .config()
        .timeout_global(Some(timeout))
        .build()
        .header("Title", title)
        .header("Priority", "high")
        .header("Tags", tag);
    if let Some(url) = &config.public_url {
        request = request.header("Click", format!("{}/#{user}", url.trim_end_matches('/')));
    }
    let response = request.send(body)?;
    if !response.status().is_success() {
        return Err(format!("ntfy returned {}", response.status()).into());
    }
    Ok(())
}

fn load_player(store: &Store, user: &str) -> Result<Player, Error> {
    Ok(Player {
        reviews: store.reviews(user)?,
        clock: store.clock(user)?,
    })
}

fn sync_users(base: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(base) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().join("collection.anki2").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

fn import(app: &App, store: &mut Store, base: &Path) -> Result<(), Error> {
    for user in sync_users(base) {
        let first_import = !store.is_known(&user)?;
        let changed = match store.ingest(base, &user) {
            Ok(changed) => changed,
            Err(e) => {
                eprintln!("ingest for {user} failed: {e}");
                continue;
            }
        };
        if changed {
            let player = load_player(store, &user)?;
            app.players.write().unwrap().insert(user.clone(), player);
        }
        if first_import && let Some(profile) = app.profile(&user) {
            for event in &profile.events {
                store.mark_seen(&user, &event.key)?;
            }
        }
    }
    Ok(())
}

fn tick(app: &App) -> Result<(), Error> {
    let mut store = app.store.lock().unwrap();
    if let Some(base) = &app.config.sync_base {
        import(app, &mut store, base)?;
    }
    let users: Vec<String> = app.players.read().unwrap().keys().cloned().collect();
    // One snapshot of the standings, so a nudge knows who is just out of reach.
    let ranking = app.profiles();
    for user in users {
        let Some(profile) = app.profile(&user) else {
            continue;
        };
        let push_enabled = push_destination(&app.config, &user).is_some();
        store.queue_events(&user, &profile.events, profile.day, now_ms(), push_enabled)?;
        if store.nudges_enabled(&user)? {
            let gap = |other: &Profile| (other.display.clone(), other.week_xp);
            let ahead = ranking
                .iter()
                .filter(|other| other.week_xp > profile.week_xp)
                .min_by_key(|other| other.week_xp)
                .map(gap);
            for nudge in game::nudges(&profile, ahead.as_ref().map(|(w, xp)| (w.as_str(), *xp))) {
                store.send_once(
                    &decks::Outgoing {
                        to: &user,
                        from: "",
                        title: &nudge.title,
                        body: &nudge.body,
                        kind: "nudge",
                    },
                    &nudge.key,
                    profile.day,
                    now_ms(),
                    false,
                )?;
            }
        }
        if profile.at_risk && profile.local_hour >= app.config.remind_hour {
            let body = format!(
                "Your {} day streak ends tonight. {}",
                profile.streak,
                if profile.freezes > 0 {
                    "A freeze would cover you, but why spend it?"
                } else {
                    "No freezes left."
                }
            );
            let key = format!("risk:{}", profile.day);
            if push_enabled {
                store.send_once(
                    &decks::Outgoing {
                        to: &user,
                        from: "",
                        title: "Streak at risk",
                        body: &body,
                        kind: "risk",
                    },
                    &key,
                    profile.day,
                    now_ms(),
                    true,
                )?;
            } else {
                store.mark_seen(&user, &key)?;
            }
        }
    }
    drop(store);
    deliver_notifications(app, PUSH_BUDGET)
}

fn deliver_notifications(app: &App, budget: Duration) -> Result<(), Error> {
    let deliveries = app.store.lock().unwrap().take_deck_deliveries(now_ms())?;
    let started = std::time::Instant::now();
    for delivery in deliveries {
        let remaining = budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            break;
        }
        let id = delivery.notification.id;
        if delivery.notification.kind == "risk"
            && app
                .profile(&delivery.user)
                .is_none_or(|profile| profile.day != delivery.notification.day || !profile.at_risk)
        {
            app.store.lock().unwrap().cancel_streak_warning(id)?;
            continue;
        }
        if push_destination(&app.config, &delivery.user).is_none() {
            app.store.lock().unwrap().defer_push(id, now_ms())?;
            continue;
        }
        if !app.store.lock().unwrap().begin_push(id, now_ms())? {
            continue;
        }
        let result = push(
            &app.config,
            &delivery.user,
            &delivery.notification.title,
            &delivery.notification.body,
            match delivery.notification.kind.as_str() {
                "event" => "tada",
                "risk" => "fire",
                _ => "white_check_mark",
            },
            remaining.min(Duration::from_secs(10)),
        );
        app.store
            .lock()
            .unwrap()
            .finish_push(id, result.is_ok(), now_ms())?;
        if let Err(e) = result {
            eprintln!(
                "ntfy push for {} failed; queued for retry: {e}",
                delivery.user
            );
        }
    }
    Ok(())
}

const USAGE: &str =
    "usage: ankiquest [config.json] [message <player> <text> [--from <player>] [--title <text>]]";

#[derive(Debug, PartialEq)]
struct Message {
    to: String,
    text: String,
    from: Option<String>,
    title: Option<String>,
}

/// Splits `[config] [message ...]` so the same binary serves and speaks.
fn parse_args(args: &[String]) -> Result<(Option<String>, Option<Message>), Error> {
    let (path, rest) = match args.first().map(String::as_str) {
        Some("message") => (None, args),
        Some(first) if !first.starts_with('-') => (Some(first.to_string()), &args[1..]),
        Some(_) => return Err(USAGE.into()),
        None => return Ok((None, None)),
    };
    let rest = match rest.first().map(String::as_str) {
        Some("message") => &rest[1..],
        Some(_) => return Err(USAGE.into()),
        None => return Ok((path, None)),
    };

    let mut positional = Vec::new();
    let mut from = None;
    let mut title = None;
    let mut rest = rest.iter();
    while let Some(argument) = rest.next() {
        let mut value = |name: &str| {
            rest.next()
                .cloned()
                .ok_or_else(|| Error::from(format!("{name} needs a value")))
        };
        match argument.as_str() {
            "--from" => from = Some(value("--from")?),
            "--title" => title = Some(value("--title")?),
            other if other.starts_with("--") => {
                return Err(format!("unknown option {other}").into());
            }
            other => positional.push(other.to_string()),
        }
    }
    let [to, text] = positional.as_slice() else {
        return Err(USAGE.into());
    };
    Ok((
        path,
        Some(Message {
            to: to.clone(),
            text: text.trim().to_string(),
            from,
            title,
        }),
    ))
}

/// Delivers a message written by hand on the server, replyable when it says who sent it.
fn send_message(config: &Config, message: &Message) -> Result<(), Error> {
    let known = |user: &str| config.users.contains_key(user);
    if !known(&message.to) {
        return Err(format!("{} is not a configured player", message.to).into());
    }
    if let Some(from) = message.from.as_deref().filter(|from| !known(from)) {
        return Err(format!("{from} is not a configured player").into());
    }
    if !decks::valid_message(&message.text) {
        return Err(format!(
            "the message must be 1 to {} characters on a single line",
            decks::MAX_MESSAGE
        )
        .into());
    }
    let title = match (&message.title, &message.from) {
        (Some(title), _) => title.clone(),
        (None, Some(from)) => format!("\u{1f4ac} {}", display_of(config, from)),
        (None, None) => "ankiquest".to_string(),
    };
    let mut store = Store::open(&config.state_dir)?;
    let now = now_ms();
    let day = store.clock(&message.to)?.day(now);
    let id = store.send(
        &decks::Outgoing {
            to: &message.to,
            from: message.from.as_deref().unwrap_or(""),
            title: &title,
            body: &message.text,
            kind: "message",
        },
        day,
        now,
    )?;
    println!("delivered to {} as notification {id}", message.to);
    Ok(())
}

fn display_of(config: &Config, user: &str) -> String {
    config
        .users
        .get(user)
        .and_then(|u| u.display.clone())
        .unwrap_or_else(|| user.to_string())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (given, message) = parse_args(&args)?;
    let path = given
        .or_else(|| std::env::var("ANKIQUEST_CONFIG").ok())
        .unwrap_or_else(|| "ankiquest.json".into());
    let mut config: Config = serde_json::from_slice(
        &std::fs::read(&path).map_err(|e| format!("cannot read config {path}: {e}"))?,
    )?;

    // Sending needs no tokens, and the running service holds the only readable copy.
    if let Some(message) = message {
        return send_message(&config, &message);
    }

    for (name, user) in &mut config.users {
        if let Some(file) = &user.token_file {
            let token = std::fs::read_to_string(file)
                .map_err(|e| format!("cannot read token for {name}: {e}"))?;
            user.token = Some(token.trim().to_string());
        }
    }

    let store = Store::open(&config.state_dir)?;
    let mut players = HashMap::new();
    for user in store.users()? {
        players.insert(user.clone(), load_player(&store, &user)?);
    }
    let week = Week {
        tz: config
            .week_timezone
            .parse()
            .map_err(|e| format!("week_timezone {:?}: {e}", config.week_timezone))?,
        rollover_hour: config.week_rollover_hour,
    };
    if week.rollover_hour > 23 {
        return Err("week_rollover_hour must be between 0 and 23".into());
    }
    let app = Arc::new(App {
        week,
        config,
        players: RwLock::new(players),
        store: Mutex::new(store),
    });

    let worker = app.clone();
    std::thread::spawn(move || {
        loop {
            if let Err(e) = tick(&worker) {
                eprintln!("tick failed: {e}");
            }
            std::thread::sleep(POLL);
        }
    });

    let router = Router::new()
        .route("/", get(index))
        .route("/records", get(index))
        .route("/{period}", get(period_page))
        .route("/manifest.webmanifest", get(manifest))
        .route("/icon.svg", get(icon))
        .route("/api/leaderboard", get(leaderboard))
        .route("/api/records", get(records))
        .route("/api/week", get(week_info))
        .route("/api/profile/{user}", get(profile))
        .route("/api/preview/{user}", post(preview))
        .route("/api/reviews/{user}", post(upload))
        .route("/api/decks/{user}", get(get_decks).post(set_decks))
        .route("/api/notifications/{user}", get(notifications))
        .route("/api/reply/{user}", post(reply))
        .with_state(app.clone());

    let listener = tokio::net::TcpListener::bind(&app.config.addr).await?;
    println!("ankiquest listening on http://{}", app.config.addr);
    axum::serve(listener, router).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Arc<App>, PathBuf) {
        let (store, path) = decks::tests::temporary_store();
        let config: Config = serde_json::from_value(serde_json::json!({
            "users": {
                "cerro": {"display": "Cerro", "token": "cerro-secret"},
                "hill": {"display": "Hill", "token": "hill-secret"},
                "friend": {"token": "friend-secret"}
            }
        }))
        .unwrap();
        (
            Arc::new(App {
                config,
                week: Week::default(),
                store: Mutex::new(store),
                players: RwLock::new(HashMap::new()),
            }),
            path,
        )
    }

    fn headers(user: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {user}-secret").parse().unwrap(),
        );
        headers
    }

    fn queued_message(app: &App, user: &str) -> i64 {
        app.store
            .lock()
            .unwrap()
            .send(
                &decks::Outgoing {
                    to: user,
                    from: "",
                    title: "Hello",
                    body: "Do not lose this",
                    kind: "message",
                },
                Clock::default().day(now_ms()),
                now_ms(),
            )
            .unwrap()
    }

    fn was_pushed(app: &App, id: i64) -> bool {
        app.store
            .lock()
            .unwrap()
            .conn
            .query_row(
                "select pushed from notifications where id = ?1",
                [id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn local_push(status: u16) -> (String, std::thread::JoinHandle<String>) {
        local_push_with_check(status, || {})
    }

    fn local_push_with_check(
        status: u16,
        check: impl FnOnce() + Send + 'static,
    ) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "no push request arrived"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("accept push: {e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 2048];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            check();
            let _ = write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            String::from_utf8(request).unwrap()
        });
        (address, server)
    }

    #[test]
    fn missing_push_configuration_does_not_consume_a_queued_message() {
        let (app, path) = fixture();
        let id = queued_message(&app, "hill");
        tick(&app).unwrap();
        assert!(
            !was_pushed(&app, id),
            "an inbox-only delivery has not been pushed"
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn stale_streak_warnings_are_cancelled_after_rollover_or_studying() {
        for studied in [false, true] {
            let (app, path) = fixture();
            let now = now_ms();
            let day = Clock::default().day(now);
            let review = Review {
                id: if studied { now } else { now - 86_400_000 },
                cid: 1,
                last_ivl: 30,
                time_ms: 5000,
                kind: 1,
            };
            {
                let mut store = app.store.lock().unwrap();
                store
                    .upsert("hill", &[review], &[], Clock::default())
                    .unwrap();
                app.players
                    .write()
                    .unwrap()
                    .insert("hill".into(), load_player(&store, "hill").unwrap());
                store
                    .send_once(
                        &decks::Outgoing {
                            to: "hill",
                            from: "",
                            title: "Streak at risk",
                            body: "Your streak ends tonight",
                            kind: "risk",
                        },
                        "risk:test",
                        if studied { day } else { day - 1 },
                        now,
                        true,
                    )
                    .unwrap();
            }
            deliver_notifications(&app, PUSH_BUDGET).unwrap();
            assert_eq!(
                app.store
                    .lock()
                    .unwrap()
                    .conn
                    .query_row(
                        "select count(*) from notifications where kind = 'risk'",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0,
                "an obsolete warning must not be sent on a later retry"
            );
            assert!(
                !app.store
                    .lock()
                    .unwrap()
                    .mark_seen("hill", "risk:test")
                    .unwrap(),
                "cancelling a stale warning must not generate it again"
            );
            drop(app);
            std::fs::remove_dir_all(path).unwrap();
        }
    }

    #[test]
    fn failed_http_push_is_not_acknowledged() {
        let (mut app, path) = fixture();
        let (base, server) = local_push(500);
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.ntfy = Some(base);
        config.users.get_mut("hill").unwrap().ntfy_topic = Some("hill-topic".into());
        let id = queued_message(&app, "hill");
        tick(&app).unwrap();
        assert!(server.join().unwrap().starts_with("POST /hill-topic "));
        assert!(
            !was_pushed(&app, id),
            "an HTTP error must leave the message retryable"
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn ntfy_pushes_request_high_priority_for_vibration_and_pop_up_alerts() {
        let (mut app, path) = fixture();
        let (base, server) = local_push(200);
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.ntfy = Some(base);
        config.users.get_mut("hill").unwrap().ntfy_topic = Some("hill-topic".into());
        let id = queued_message(&app, "hill");
        deliver_notifications(&app, PUSH_BUDGET).unwrap();
        let request = server.join().unwrap();
        assert!(
            request
                .lines()
                .any(|line| line.eq_ignore_ascii_case("priority: high")),
            "ntfy needs high priority to request vibration and a pop-up"
        );
        assert!(was_pushed(&app, id));
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn successful_retry_acknowledges_the_same_message_without_holding_the_store_lock() {
        let (mut app, path) = fixture();
        let (base, failed) = local_push(503);
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.ntfy = Some(base);
        config.users.get_mut("hill").unwrap().ntfy_topic = Some("hill-topic".into());
        let id = queued_message(&app, "hill");
        deliver_notifications(&app, PUSH_BUDGET).unwrap();
        failed.join().unwrap();
        assert!(!was_pushed(&app, id));
        let app_for_check = Arc::new(Mutex::new(std::sync::Weak::<App>::new()));
        let check = app_for_check.clone();
        let (base, success) = local_push_with_check(200, move || {
            let app = check.lock().unwrap().upgrade().unwrap();
            assert!(
                app.store.try_lock().is_ok(),
                "HTTP must not block uploads or inbox reads"
            );
        });
        Arc::get_mut(&mut app).unwrap().config.ntfy = Some(base);
        *app_for_check.lock().unwrap() = Arc::downgrade(&app);
        app.store
            .lock()
            .unwrap()
            .conn
            .execute("update notifications set retry_at = 0 where id = ?1", [id])
            .unwrap();
        deliver_notifications(&app, PUSH_BUDGET).unwrap();
        let request = success.join().unwrap();
        assert!(request.starts_with("POST /hill-topic "));
        assert!(request.ends_with("Do not lose this"));
        assert!(was_pushed(&app, id));
        deliver_notifications(&app, PUSH_BUDGET).unwrap();
        let store = app.store.lock().unwrap();
        assert_eq!(store.notifications("hill", now_ms()).unwrap()[0].id, id);
        assert_eq!(
            store
                .conn
                .query_row(
                    "select push_attempts from notifications where id = ?1",
                    [id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            2
        );
        drop(store);
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn unconfigured_front_rows_do_not_block_a_configured_recipient() {
        let (mut app, path) = fixture();
        let (base, server) = local_push(200);
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.ntfy = Some(base);
        config.users.get_mut("friend").unwrap().ntfy_topic = Some("friend-topic".into());
        for _ in 0..101 {
            queued_message(&app, "hill");
        }
        let id = queued_message(&app, "friend");
        deliver_notifications(&app, PUSH_BUDGET).unwrap();
        assert!(server.join().unwrap().starts_with("POST /friend-topic "));
        assert!(was_pushed(&app, id));
        assert_eq!(app.store.lock().unwrap().conn.query_row("select sum(push_attempts + pushed) from notifications where recipient = 'hill'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn network_budget_leaves_unattempted_messages_ready_for_the_next_tick() {
        let (mut app, path) = fixture();
        let (base, server) =
            local_push_with_check(200, || std::thread::sleep(Duration::from_millis(500)));
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.ntfy = Some(base);
        for user in ["hill", "friend"] {
            config.users.get_mut(user).unwrap().ntfy_topic = Some(user.into());
        }
        let first = queued_message(&app, "hill");
        let second = queued_message(&app, "friend");
        deliver_notifications(&app, Duration::ZERO).unwrap();
        let start = std::time::Instant::now();
        deliver_notifications(&app, Duration::from_millis(100)).unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "the request timeout respects the remaining budget"
        );
        server.join().unwrap();
        assert!(!was_pushed(&app, first));
        let store = app.store.lock().unwrap();
        assert_eq!(
            store
                .conn
                .query_row(
                    "select push_attempts + retry_at from notifications where id = ?1",
                    [second],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0,
            "skipped messages must not be leased or marked delivered"
        );
        drop(store);
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn failed_server_game_events_remain_durable_without_duplicating_the_inbox() {
        let (mut app, path) = fixture();
        let (base, server) = local_push(500);
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.ntfy = Some(base);
        config.users.get_mut("cerro").unwrap().ntfy_topic = Some("cerro-topic".into());
        let now = now_ms();
        let reviews: Vec<_> = (0..100)
            .map(|i| Review {
                id: now - (100 - i) * 10_000,
                cid: i,
                last_ivl: 30,
                time_ms: 5000,
                kind: 0,
            })
            .collect();
        {
            let mut store = app.store.lock().unwrap();
            store
                .upsert("cerro", &reviews, &[], Clock::default())
                .unwrap();
            app.players
                .write()
                .unwrap()
                .insert("cerro".into(), load_player(&store, "cerro").unwrap());
        }
        assert!(!app.profile("cerro").unwrap().events.is_empty());
        tick(&app).unwrap();
        server.join().unwrap();
        let count: i64 = app
            .store
            .lock()
            .unwrap()
            .conn
            .query_row(
                "select count(*) from notifications where recipient = 'cerro' and pushed = 0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0, "failed server unlocks need a durable retry");
        tick(&app).unwrap();
        assert!(
            app.store
                .lock()
                .unwrap()
                .notifications("cerro", now_ms())
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            app.store
                .lock()
                .unwrap()
                .conn
                .query_row(
                    "select count(*) from notifications where recipient = 'cerro'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            count
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    fn sample_upload(remaining: u64, silent: bool) -> Upload {
        Upload {
            reviews: vec![],
            deleted: vec![],
            clock: Clock::default(),
            silent,
            catalog: false,
            decks: Some(vec![decks::Snapshot {
                id: "1".into(),
                name: "Spanish".into(),
                remaining,
                reviewed_today: 3,
                day: Clock::default().day(now_ms()),
            }]),
        }
    }

    fn preferences(recipients: &[&str]) -> decks::SettingsUpdate {
        decks::SettingsUpdate {
            decks: vec![decks::Preference {
                id: "1".into(),
                enabled: true,
                recipients: recipients.iter().map(|r| (*r).into()).collect(),
            }],
            nudges: None,
        }
    }

    #[test]
    fn empty_configured_token_never_authorizes_private_data() {
        let (mut app, path) = fixture();
        Arc::get_mut(&mut app)
            .unwrap()
            .config
            .users
            .get_mut("cerro")
            .unwrap()
            .token = Some(String::new());
        assert!(!authorized(&app, "cerro", &HeaderMap::new()));
        let mut empty_bearer = HeaderMap::new();
        empty_bearer.insert(header::AUTHORIZATION, "Bearer ".parse().unwrap());
        assert!(!authorized(&app, "cerro", &empty_bearer));
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn recipient_choices_require_a_configured_delivery_channel() {
        let (mut app, path) = fixture();
        let config = &mut Arc::get_mut(&mut app).unwrap().config;
        config.ntfy = Some("http://localhost:9999".into());
        config.users.insert(
            "push-only".into(),
            UserConfig {
                ntfy_topic: Some("friend-topic".into()),
                ..UserConfig::default()
            },
        );
        config.users.insert(
            "empty-token".into(),
            UserConfig {
                token: Some(String::new()),
                ..UserConfig::default()
            },
        );
        config
            .users
            .insert("display-only".into(), UserConfig::default());
        config.users.insert(
            "empty-topic".into(),
            UserConfig {
                ntfy_topic: Some(String::new()),
                ..UserConfig::default()
            },
        );
        {
            let mut store = app.store.lock().unwrap();
            store
                .upsert("imported", &[], &[], Clock::default())
                .unwrap();
            let settings = deck_settings(&app, &store, "cerro").unwrap();
            assert_eq!(
                settings
                    .recipients
                    .iter()
                    .map(|r| r.user.as_str())
                    .collect::<Vec<_>>(),
                vec!["friend", "hill", "push-only"]
            );
            assert!(settings.decks.is_empty());
        }
        Arc::get_mut(&mut app).unwrap().config.ntfy = None;
        assert_eq!(
            deck_settings(&app, &app.store.lock().unwrap(), "cerro")
                .unwrap()
                .recipients
                .iter()
                .map(|r| r.user.as_str())
                .collect::<Vec<_>>(),
            vec!["friend", "hill"]
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn all_private_handlers_reject_missing_and_other_users_tokens() {
        let (app, path) = fixture();
        for auth in [HeaderMap::new(), headers("hill")] {
            assert_eq!(
                get_decks(State(app.clone()), UrlPath("cerro".into()), auth.clone())
                    .await
                    .unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                set_decks(
                    State(app.clone()),
                    UrlPath("cerro".into()),
                    auth.clone(),
                    Json(preferences(&["hill"]))
                )
                .await
                .unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                notifications(State(app.clone()), UrlPath("cerro".into()), auth.clone())
                    .await
                    .unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                reply(
                    State(app.clone()),
                    UrlPath("cerro".into()),
                    auth.clone(),
                    Json(ReplyRequest {
                        notification: 1,
                        message: "Good job!".into()
                    })
                )
                .await
                .unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
            assert_eq!(
                upload(
                    State(app.clone()),
                    UrlPath("cerro".into()),
                    auth,
                    Json(sample_upload(0, false))
                )
                .await
                .unwrap_err(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert!(app.store.lock().unwrap().decks("cerro").unwrap().is_empty());
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn the_upload_reports_what_it_just_announced() {
        let (app, path) = fixture();
        let send = |body: Upload| {
            let app = app.clone();
            async move {
                upload(
                    State(app),
                    UrlPath("cerro".into()),
                    headers("cerro"),
                    Json(body),
                )
                .await
                .unwrap()
                .0
            }
        };
        assert!(send(sample_upload(2, false)).await.announced.is_empty());
        let _ = set_decks(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(preferences(&["hill"])),
        )
        .await
        .unwrap();

        let announced = send(sample_upload(0, false)).await.announced;
        assert_eq!(announced.len(), 1);
        assert_eq!(announced[0].deck, "Spanish");
        assert_eq!(announced[0].recipients, 1);
        assert!(
            send(sample_upload(0, false)).await.announced.is_empty(),
            "a deck is only announced once a day"
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    fn words(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_string()).collect()
    }

    fn studied(user: &str, count: i64, day: i64) -> Upload {
        Upload {
            reviews: (0..count)
                .map(|index| Review {
                    id: day * 86_400_000 + 12 * 3_600_000 + index * 60_000 + user.len() as i64,
                    cid: index + 1,
                    last_ivl: 30,
                    time_ms: 5_000,
                    kind: 1,
                })
                .collect(),
            deleted: vec![],
            clock: Clock::default(),
            silent: true,
            catalog: false,
            decks: None,
        }
    }

    #[tokio::test]
    async fn the_records_board_names_whoever_had_the_best_window() {
        let (app, path) = fixture();
        let day = Clock::default().day(now_ms());
        for (user, count) in [("cerro", 30), ("hill", 8)] {
            let _ = upload(
                State(app.clone()),
                UrlPath(user.into()),
                headers(user),
                Json(studied(user, count, day)),
            )
            .await
            .unwrap();
        }
        let board = records(State(app.clone())).await.0;
        assert_eq!(
            board.iter().map(|r| r.window.as_str()).collect::<Vec<_>>(),
            [Records::NAMES.as_slice(), &["streak", "days"]].concat()
        );
        for window in board.iter().filter(|window| window.unit == "xp") {
            assert_eq!(
                window
                    .holders
                    .iter()
                    .map(|h| h.user.as_str())
                    .collect::<Vec<_>>(),
                vec!["cerro", "hill"],
                "{} lists the holder and whoever came closest",
                window.window
            );
            assert_eq!(window.holders[0].display, "Cerro");
            assert!(window.holders[0].value > window.holders[1].value);
            assert!(window.holders[1].detail > 0);
        }
        assert_eq!(
            board[0].holders[0].detail, 30,
            "all thirty land inside one hour"
        );
        let lifetime: Vec<&RecordBoard> = board
            .iter()
            .filter(|window| window.unit == "days")
            .collect();
        assert_eq!(
            lifetime
                .iter()
                .map(|w| w.window.as_str())
                .collect::<Vec<_>>(),
            vec!["streak", "days"]
        );
        for window in lifetime {
            assert_eq!(
                window.holders.len(),
                2,
                "{} names everyone who has studied",
                window.window
            );
            assert!(
                window
                    .holders
                    .iter()
                    .all(|holder| holder.value == 1 && holder.detail == 0)
            );
            assert!(window.holders.iter().all(|holder| holder.at > 0));
        }
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn nudges_are_off_until_asked_for_and_then_stay_on() {
        let (app, path) = fixture();
        let _ = upload(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(sample_upload(2, false)),
        )
        .await
        .unwrap();
        let save = |nudges: Option<bool>| {
            let app = app.clone();
            async move {
                let mut update = preferences(&["hill"]);
                update.nudges = nudges;
                set_decks(
                    State(app),
                    UrlPath("cerro".into()),
                    headers("cerro"),
                    Json(update),
                )
                .await
                .unwrap()
                .0
            }
        };
        assert!(!save(None).await.nudges, "off until someone asks");
        assert!(save(Some(true)).await.nudges);
        assert!(save(None).await.nudges, "an older client leaves it alone");
        assert!(!save(Some(false)).await.nudges);
        assert!(
            !get_decks(State(app.clone()), UrlPath("hill".into()), headers("hill"))
                .await
                .unwrap()
                .0
                .nudges,
            "the setting belongs to one player"
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn the_message_command_reads_a_config_and_its_options_in_any_order() {
        assert_eq!(parse_args(&words(&[])).unwrap(), (None, None));
        let (path, nothing) = parse_args(&words(&["/etc/ankiquest.json"])).unwrap();
        assert_eq!(path.as_deref(), Some("/etc/ankiquest.json"));
        assert_eq!(nothing, None);

        let (path, message) = parse_args(&words(&[
            "/etc/ankiquest.json",
            "message",
            "--from",
            "cerro",
            "aldanita",
            "  keep going  ",
            "--title",
            "Hi",
        ]))
        .unwrap();
        assert_eq!(path.as_deref(), Some("/etc/ankiquest.json"));
        assert_eq!(
            message,
            Some(Message {
                to: "aldanita".into(),
                text: "keep going".into(),
                from: Some("cerro".into()),
                title: Some("Hi".into()),
            })
        );

        let (path, message) = parse_args(&words(&["message", "aldanita", "hello"])).unwrap();
        assert_eq!(path, None, "the config then comes from the environment");
        assert_eq!(message.unwrap().from, None);

        for bad in [
            vec!["message"],
            vec!["message", "aldanita"],
            vec!["message", "aldanita", "hello", "spare"],
            vec!["message", "aldanita", "hello", "--from"],
            vec!["message", "aldanita", "hello", "--shout"],
            vec!["--help"],
            vec!["/etc/ankiquest.json", "serve"],
        ] {
            assert!(
                parse_args(&words(&bad)).is_err(),
                "{bad:?} should not parse"
            );
        }
    }

    #[tokio::test]
    async fn a_message_written_on_the_server_lands_in_the_inbox_and_can_be_answered() {
        let (app, path) = fixture();
        let config: Config = serde_json::from_value(serde_json::json!({
            "state_dir": path,
            "users": {
                "cerro": {"display": "Cerro", "token": "cerro-secret"},
                "hill": {"display": "Hill", "token": "hill-secret"},
            }
        }))
        .unwrap();
        let note = |to: &str, from: Option<&str>| Message {
            to: to.into(),
            text: "you're doing great, keep going".into(),
            from: from.map(str::to_string),
            title: None,
        };
        send_message(&config, &note("hill", Some("cerro"))).unwrap();

        let inbox = notifications(State(app.clone()), UrlPath("hill".into()), headers("hill"))
            .await
            .unwrap()
            .0;
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].title, "\u{1f4ac} Cerro");
        assert_eq!(inbox[0].kind, "message", "written by hand, not by the game");
        assert_eq!(inbox[0].body, "you're doing great, keep going");
        assert_eq!(inbox[0].sender, "cerro");
        assert_eq!(
            reply(
                State(app.clone()),
                UrlPath("hill".into()),
                headers("hill"),
                Json(ReplyRequest {
                    notification: inbox[0].id,
                    message: "thank you!".into()
                })
            )
            .await
            .unwrap()
            .0
            .sent_to,
            "Cerro"
        );

        send_message(&config, &note("hill", None)).unwrap();
        let anonymous = notifications(State(app.clone()), UrlPath("hill".into()), headers("hill"))
            .await
            .unwrap()
            .0
            .pop()
            .unwrap();
        assert_eq!(anonymous.title, "ankiquest");
        assert!(anonymous.sender.is_empty(), "nobody to answer");

        assert!(send_message(&config, &note("nobody", None)).is_err());
        assert!(send_message(&config, &note("hill", Some("nobody"))).is_err());
        for text in ["", "   ", "two\nlines", &"x".repeat(decks::MAX_MESSAGE + 1)] {
            let mut empty = note("hill", None);
            empty.text = text.into();
            assert!(
                send_message(&config, &empty).is_err(),
                "{text:?} is not a message"
            );
        }
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn a_completion_can_be_answered_once_and_the_answer_answered_back() {
        let (app, path) = fixture();
        let post = |app: Arc<App>, body: Upload| async move {
            let _ = upload(
                State(app),
                UrlPath("cerro".into()),
                headers("cerro"),
                Json(body),
            )
            .await
            .unwrap();
        };
        let inbox = |app: Arc<App>, user: &'static str| async move {
            notifications(State(app), UrlPath(user.into()), headers(user))
                .await
                .unwrap()
                .0
        };
        let say = |app: Arc<App>, user: &'static str, notification: i64, message: &str| {
            let message = message.to_string();
            async move {
                reply(
                    State(app),
                    UrlPath(user.into()),
                    headers(user),
                    Json(ReplyRequest {
                        notification,
                        message,
                    }),
                )
                .await
            }
        };

        post(app.clone(), sample_upload(2, false)).await;
        let _ = set_decks(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(preferences(&["hill"])),
        )
        .await
        .unwrap();
        post(app.clone(), sample_upload(0, false)).await;

        let completion = inbox(app.clone(), "hill").await.remove(0);
        assert_eq!(completion.sender, "cerro");
        assert!(!completion.replied);
        let id = completion.id;
        assert_eq!(
            say(app.clone(), "hill", id, "  Good job!  ")
                .await
                .unwrap()
                .0
                .sent_to,
            "Cerro"
        );
        assert!(
            inbox(app.clone(), "hill").await[0].replied,
            "a reply is only sent once"
        );

        let answer = inbox(app.clone(), "cerro").await.remove(0);
        assert_eq!(answer.title, "\u{1f4ac} Hill");
        assert_eq!(answer.body, "Good job!");
        assert_eq!(answer.sender, "hill");
        assert_eq!(
            say(app.clone(), "cerro", answer.id, "thanks!")
                .await
                .unwrap()
                .0
                .sent_to,
            "Hill"
        );
        assert_eq!(inbox(app.clone(), "hill").await[1].body, "thanks!");

        for (user, notification, message) in [
            ("hill", id, "already answered"),
            ("friend", id, "not my notification"),
            ("cerro", answer.id, "answered by me"),
            ("hill", id + 999, "no such notification"),
        ] {
            assert_eq!(
                say(app.clone(), user, notification, message)
                    .await
                    .unwrap_err(),
                StatusCode::NOT_FOUND
            );
        }
        for message in [
            "",
            "   ",
            "line\nbreak",
            &"x".repeat(decks::MAX_MESSAGE + 1),
        ] {
            assert_eq!(
                say(app.clone(), "hill", id, message).await.unwrap_err(),
                StatusCode::BAD_REQUEST
            );
        }
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn authenticated_upload_preferences_and_inbox_end_to_end() {
        let (app, path) = fixture();
        let _ = upload(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(sample_upload(2, false)),
        )
        .await
        .unwrap();
        let settings = get_decks(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
        )
        .await
        .unwrap()
        .0;
        assert!(!settings.decks[0].enabled);
        assert_eq!(settings.decks[0].name, "Spanish");
        assert_eq!(
            settings
                .recipients
                .iter()
                .map(|r| r.user.as_str())
                .collect::<Vec<_>>(),
            vec!["friend", "hill"]
        );
        assert_eq!(settings.recipients[1].display, "Hill");
        let _ = set_decks(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(preferences(&["hill"])),
        )
        .await
        .unwrap();
        for _ in 0..2 {
            let _ = upload(
                State(app.clone()),
                UrlPath("cerro".into()),
                headers("cerro"),
                Json(sample_upload(0, false)),
            )
            .await
            .unwrap();
        }
        let inbox = notifications(State(app.clone()), UrlPath("hill".into()), headers("hill"))
            .await
            .unwrap()
            .0;
        assert_eq!(inbox.len(), 1);
        assert_eq!(
            inbox[0].body,
            "Cerro has finished their Spanish studies for today."
        );
        assert!(inbox[0].created_at <= now_ms() / 1000);
        assert!(
            notifications(
                State(app.clone()),
                UrlPath("friend".into()),
                headers("friend")
            )
            .await
            .unwrap()
            .0
            .is_empty()
        );
        assert!(
            get_decks(State(app.clone()), UrlPath("hill".into()), headers("hill"))
                .await
                .unwrap()
                .0
                .decks
                .is_empty()
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn rejects_invalid_updates_without_changing_settings() {
        let (app, path) = fixture();
        let _ = upload(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(sample_upload(2, false)),
        )
        .await
        .unwrap();
        for update in [
            preferences(&["cerro"]),
            preferences(&["unknown"]),
            preferences(&[]),
            preferences(&["hill", "hill"]),
            decks::SettingsUpdate {
                decks: vec![decks::Preference {
                    id: "missing".into(),
                    enabled: false,
                    recipients: vec![],
                }],
                nudges: None,
            },
            decks::SettingsUpdate {
                decks: vec![
                    decks::Preference {
                        id: "1".into(),
                        enabled: true,
                        recipients: vec!["hill".into()],
                    },
                    decks::Preference {
                        id: "1".into(),
                        enabled: false,
                        recipients: vec![],
                    },
                ],
                nudges: None,
            },
        ] {
            assert_eq!(
                set_decks(
                    State(app.clone()),
                    UrlPath("cerro".into()),
                    headers("cerro"),
                    Json(update)
                )
                .await
                .unwrap_err(),
                StatusCode::BAD_REQUEST
            );
        }
        let settings = get_decks(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
        )
        .await
        .unwrap()
        .0;
        assert!(!settings.decks[0].enabled);
        assert!(settings.decks[0].recipients.is_empty());
        let mut bad = sample_upload(0, false);
        bad.clock.offset_west_min = i64::MAX;
        assert_eq!(
            upload(
                State(app.clone()),
                UrlPath("cerro".into()),
                headers("cerro"),
                Json(bad)
            )
            .await
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );
        let mut duplicate = sample_upload(0, false);
        let repeated = duplicate.decks.as_ref().unwrap()[0].clone();
        duplicate.decks.as_mut().unwrap().push(repeated);
        assert_eq!(
            upload(
                State(app.clone()),
                UrlPath("cerro".into()),
                headers("cerro"),
                Json(duplicate)
            )
            .await
            .unwrap_err(),
            StatusCode::BAD_REQUEST
        );
        let mut oversized = sample_upload(0, false);
        let deck = oversized.decks.as_ref().unwrap()[0].clone();
        oversized.decks = Some(vec![deck; decks::MAX_DECKS + 1]);
        assert_eq!(
            upload(
                State(app.clone()),
                UrlPath("cerro".into()),
                headers("cerro"),
                Json(oversized)
            )
            .await
            .unwrap_err(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn legacy_and_silent_uploads_and_previews_do_not_publish_completion() {
        let (app, path) = fixture();
        let legacy: Upload = serde_json::from_value(serde_json::json!({
            "reviews": [], "clock": {"offset_west_min": 0, "rollover_hour": 4}
        }))
        .unwrap();
        assert!(legacy.decks.is_none());
        let _ = upload(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(legacy),
        )
        .await
        .unwrap();
        let _ = upload(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(sample_upload(2, false)),
        )
        .await
        .unwrap();
        let _ = set_decks(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(preferences(&["hill"])),
        )
        .await
        .unwrap();
        let _ = upload(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(sample_upload(0, true)),
        )
        .await
        .unwrap();
        let _ = upload(
            State(app.clone()),
            UrlPath("cerro".into()),
            headers("cerro"),
            Json(sample_upload(0, false)),
        )
        .await
        .unwrap();
        let _ = preview(
            State(app.clone()),
            UrlPath("cerro".into()),
            Json(Pending { reviews: vec![] }),
        )
        .await
        .unwrap();
        assert!(
            notifications(State(app.clone()), UrlPath("hill".into()), headers("hill"))
                .await
                .unwrap()
                .0
                .is_empty()
        );
        drop(app);
        std::fs::remove_dir_all(path).unwrap();
    }
}
