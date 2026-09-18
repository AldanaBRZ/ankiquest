mod game;
mod store;

use axum::extract::{Path as UrlPath, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use game::{Clock, Profile, Review};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use store::{Error, Store};

const POLL: Duration = Duration::from_secs(20);
const MAX_PENDING: usize = 5000;
const MAX_SEPARATE_PUSHES: usize = 3;

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
    #[serde(default)]
    users: HashMap<String, UserConfig>,
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
    streak: u64,
    today_reviews: u64,
}

async fn leaderboard(State(app): State<Arc<App>>) -> Json<Vec<Standing>> {
    let mut standings: Vec<Standing> = app
        .profiles()
        .into_iter()
        .map(|p| Standing {
            user: p.user,
            display: p.display,
            level: p.level,
            xp_total: p.xp_total,
            week_xp: p.week_xp,
            streak: p.streak,
            today_reviews: p.today.reviews,
        })
        .collect();
    standings.sort_by_key(|s| std::cmp::Reverse((s.week_xp, s.xp_total)));
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
    given.len() == expected.len()
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
) -> Result<Json<Profile>, StatusCode> {
    if !authorized(&app, &user, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if upload.reviews.len() > MAX_PENDING || upload.deleted.len() > MAX_PENDING {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
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
    let player = load_player(&store, &user).map_err(store_error)?;
    app.players.write().unwrap().insert(user.clone(), player);
    let profile = app.profile(&user).ok_or(StatusCode::NOT_FOUND)?;
    if silent {
        for event in &profile.events {
            store.mark_seen(&user, &event.key).map_err(store_error)?;
        }
    }
    Ok(Json(profile))
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
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

fn push(config: &Config, user: &str, title: &str, body: &str, tag: &str) {
    let (Some(base), Some(topic)) = (
        config.ntfy.as_deref(),
        config.users.get(user).and_then(|u| u.ntfy_topic.as_deref()),
    ) else {
        return;
    };
    let mut request = ureq::post(&format!("{}/{topic}", base.trim_end_matches('/')))
        .timeout(Duration::from_secs(10))
        .set("Title", title)
        .set("Tags", tag);
    if let Some(url) = &config.public_url {
        request = request.set("Click", &format!("{}/#{user}", url.trim_end_matches('/')));
    }
    if let Err(e) = request.send_string(body) {
        eprintln!("ntfy push for {user} failed: {e}");
    }
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
    for user in users {
        let Some(profile) = app.profile(&user) else {
            continue;
        };
        let mut fresh = Vec::new();
        for event in &profile.events {
            if store.mark_seen(&user, &event.key)? {
                fresh.push(event);
            }
        }
        if fresh.len() > MAX_SEPARATE_PUSHES {
            let titles: Vec<&str> = fresh.iter().map(|e| e.title.as_str()).collect();
            let title = format!("{} new unlocks", fresh.len());
            push(&app.config, &user, &title, &titles.join(", "), "tada");
        } else {
            for event in fresh {
                push(&app.config, &user, &event.title, &event.body, "tada");
            }
        }
        if profile.at_risk
            && profile.local_hour >= app.config.remind_hour
            && store.mark_seen(&user, &format!("risk:{}", profile.day))?
        {
            let body = format!(
                "Your {} day streak ends tonight. {}",
                profile.streak,
                if profile.freezes > 0 {
                    "A freeze would cover you, but why spend it?"
                } else {
                    "No freezes left."
                }
            );
            push(&app.config, &user, "Streak at risk", &body, "fire");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("ANKIQUEST_CONFIG").ok())
        .unwrap_or_else(|| "ankiquest.json".into());
    let mut config: Config = serde_json::from_slice(
        &std::fs::read(&path).map_err(|e| format!("cannot read config {path}: {e}"))?,
    )?;

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
    let app = Arc::new(App {
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
        .route("/manifest.webmanifest", get(manifest))
        .route("/icon.svg", get(icon))
        .route("/api/leaderboard", get(leaderboard))
        .route("/api/profile/{user}", get(profile))
        .route("/api/preview/{user}", post(preview))
        .route("/api/reviews/{user}", post(upload))
        .with_state(app.clone());

    let listener = tokio::net::TcpListener::bind(&app.config.addr).await?;
    println!("ankiquest listening on http://{}", app.config.addr);
    axum::serve(listener, router).await?;
    Ok(())
}
