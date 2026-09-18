mod game;
mod store;

use axum::extract::{Path as UrlPath, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::{Json, Router};
use game::{Clock, Profile, Review};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use store::{Error, Store};

const POLL: Duration = Duration::from_secs(20);

#[derive(Deserialize, Clone, Default)]
struct UserConfig {
    display: Option<String>,
    ntfy_topic: Option<String>,
}

#[derive(Deserialize, Clone)]
struct Config {
    #[serde(default = "default_addr")]
    addr: String,
    sync_base: PathBuf,
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
        let players = self.players.read().unwrap();
        let player = players.get(user)?;
        Some(game::compute(
            user,
            &self.display(user),
            &player.reviews,
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

fn sync_users(config: &Config) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(&config.sync_base) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.path().join("collection.anki2").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

fn tick(app: &App, store: &mut Store) -> Result<(), Error> {
    for user in sync_users(&app.config) {
        let first_import = !store.is_known(&user)?;
        let changed = match store.ingest(&app.config.sync_base, &user) {
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
        let Some(profile) = app.profile(&user) else {
            continue;
        };
        for event in &profile.events {
            if store.mark_seen(&user, &event.key)? && !first_import {
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
    let config: Config = serde_json::from_slice(
        &std::fs::read(&path).map_err(|e| format!("cannot read config {path}: {e}"))?,
    )?;

    let mut store = Store::open(&config.state_dir)?;
    let mut players = HashMap::new();
    for user in store.users()? {
        players.insert(user.clone(), load_player(&store, &user)?);
    }
    let app = Arc::new(App {
        config,
        players: RwLock::new(players),
    });

    let worker = app.clone();
    std::thread::spawn(move || {
        loop {
            if let Err(e) = tick(&worker, &mut store) {
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
        .with_state(app.clone());

    let listener = tokio::net::TcpListener::bind(&app.config.addr).await?;
    println!("ankiquest listening on http://{}", app.config.addr);
    axum::serve(listener, router).await?;
    Ok(())
}
