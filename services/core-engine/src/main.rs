use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::Json,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use uuid::Uuid;

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Serialize)]
struct Message {
    id: String,
    room_id: String,
    sender_id: String,
    content: String,
    timestamp: u64,
    msg_type: String,
}

#[derive(Debug, Clone, Serialize)]
struct Room {
    id: String,
    name: String,
    members: Vec<String>,
    created_at: u64,
}

#[derive(Debug, Default)]
struct Stats {
    total_ops: u64,
    send_ops: u64,
    room_ops: u64,
}

#[derive(Debug, Default)]
struct AppData {
    stats: Stats,
    messages: HashMap<String, Vec<Message>>, // room_id -> messages
    rooms: HashMap<String, Room>,
    presence: HashMap<String, u64>, // user_id -> last_seen ts
}

type AppState = Arc<Mutex<AppData>>;

// --- /api/v1/chat/send ---
#[derive(Debug, Deserialize)]
struct SendRequest {
    room_id: String,
    sender_id: String,
    content: String,
    #[serde(default = "default_msg_type")]
    msg_type: String,
}

fn default_msg_type() -> String {
    "text".to_string()
}

#[derive(Debug, Serialize)]
struct SendResponse {
    request_id: String,
    message_id: String,
    room_id: String,
    timestamp: u64,
    delivered: bool,
}

async fn send_message(
    State(state): State<AppState>,
    Json(req): Json<SendRequest>,
) -> Result<Json<SendResponse>, StatusCode> {
    let msg_id = Uuid::new_v4().to_string();
    let ts = now_ts();
    let msg = Message {
        id: msg_id.clone(),
        room_id: req.room_id.clone(),
        sender_id: req.sender_id.clone(),
        content: req.content,
        timestamp: ts,
        msg_type: req.msg_type,
    };
    {
        let mut d = state.lock().unwrap();
        d.stats.total_ops += 1;
        d.stats.send_ops += 1;
        d.messages
            .entry(req.room_id.clone())
            .or_default()
            .push(msg);
        d.presence.insert(req.sender_id, ts);
    }
    Ok(Json(SendResponse {
        request_id: Uuid::new_v4().to_string(),
        message_id: msg_id,
        room_id: req.room_id,
        timestamp: ts,
        delivered: true,
    }))
}

// --- /api/v1/chat/rooms ---
#[derive(Debug, Deserialize)]
struct RoomRequest {
    name: String,
    creator_id: String,
    #[serde(default)]
    initial_members: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RoomResponse {
    request_id: String,
    room_id: String,
    name: String,
    members: Vec<String>,
    created_at: u64,
}

async fn create_room(
    State(state): State<AppState>,
    Json(req): Json<RoomRequest>,
) -> Result<Json<RoomResponse>, StatusCode> {
    let room_id = Uuid::new_v4().to_string();
    let ts = now_ts();
    let mut members = req.initial_members.clone();
    if !members.contains(&req.creator_id) {
        members.push(req.creator_id.clone());
    }
    let room = Room {
        id: room_id.clone(),
        name: req.name.clone(),
        members: members.clone(),
        created_at: ts,
    };
    {
        let mut d = state.lock().unwrap();
        d.stats.total_ops += 1;
        d.stats.room_ops += 1;
        d.rooms.insert(room_id.clone(), room);
        d.presence.insert(req.creator_id, ts);
    }
    Ok(Json(RoomResponse {
        request_id: Uuid::new_v4().to_string(),
        room_id,
        name: req.name,
        members,
        created_at: ts,
    }))
}

// --- /api/v1/chat/history ---
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    room_id: String,
    #[serde(default = "default_limit")]
    limit: usize,
    before_ts: Option<u64>,
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Serialize)]
struct HistoryResponse {
    room_id: String,
    messages: Vec<Message>,
    count: usize,
    has_more: bool,
}

async fn get_history(
    State(state): State<AppState>,
    Query(q): Query<HistoryQuery>,
) -> Json<HistoryResponse> {
    let d = state.lock().unwrap();
    let all = d.messages.get(&q.room_id).cloned().unwrap_or_default();
    let filtered: Vec<Message> = all
        .into_iter()
        .filter(|m| q.before_ts.map_or(true, |ts| m.timestamp < ts))
        .collect();
    let total = filtered.len();
    let has_more = total > q.limit;
    let messages: Vec<Message> = filtered.into_iter().rev().take(q.limit).collect();
    let count = messages.len();
    Json(HistoryResponse {
        room_id: q.room_id,
        messages,
        count,
        has_more,
    })
}

// --- /api/v1/chat/presence ---
#[derive(Debug, Deserialize)]
struct PresenceQuery {
    user_ids: Option<String>, // comma-separated
}

#[derive(Debug, Serialize)]
struct PresenceEntry {
    user_id: String,
    online: bool,
    last_seen: u64,
}

#[derive(Debug, Serialize)]
struct PresenceResponse {
    users: Vec<PresenceEntry>,
    online_count: usize,
}

async fn get_presence(
    State(state): State<AppState>,
    Query(q): Query<PresenceQuery>,
) -> Json<PresenceResponse> {
    let d = state.lock().unwrap();
    let now = now_ts();
    let online_threshold = 300u64; // 5 minutes

    let users: Vec<PresenceEntry> = if let Some(ids) = q.user_ids {
        ids.split(',')
            .map(|uid| {
                let uid = uid.trim().to_string();
                let last_seen = d.presence.get(&uid).copied().unwrap_or(0);
                let online = last_seen > 0 && now.saturating_sub(last_seen) < online_threshold;
                PresenceEntry { user_id: uid, online, last_seen }
            })
            .collect()
    } else {
        d.presence
            .iter()
            .map(|(uid, &ts)| {
                let online = now.saturating_sub(ts) < online_threshold;
                PresenceEntry { user_id: uid.clone(), online, last_seen: ts }
            })
            .collect()
    };

    let online_count = users.iter().filter(|u| u.online).count();
    Json(PresenceResponse { users, online_count })
}

// --- /api/v1/chat/stats ---
#[derive(Debug, Serialize)]
struct StatsResponse {
    service: &'static str,
    version: &'static str,
    total_ops: u64,
    send_ops: u64,
    room_ops: u64,
    total_rooms: usize,
    total_messages: usize,
    tracked_users: usize,
}

async fn get_stats(State(state): State<AppState>) -> Json<StatsResponse> {
    let d = state.lock().unwrap();
    let total_messages = d.messages.values().map(|v| v.len()).sum();
    Json(StatsResponse {
        service: "alice-chat-core",
        version: env!("CARGO_PKG_VERSION"),
        total_ops: d.stats.total_ops,
        send_ops: d.stats.send_ops,
        room_ops: d.stats.room_ops,
        total_rooms: d.rooms.len(),
        total_messages,
        tracked_users: d.presence.len(),
    })
}

// --- /health ---
#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    uptime_secs: u64,
    total_ops: u64,
}

async fn health(
    State(state): State<AppState>,
    axum::extract::Extension(start): axum::extract::Extension<Arc<Instant>>,
) -> Json<HealthResponse> {
    let d = state.lock().unwrap();
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: start.elapsed().as_secs(),
        total_ops: d.stats.total_ops,
    })
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let state: AppState = Arc::new(Mutex::new(AppData::default()));
    let start = Arc::new(Instant::now());

    let app = Router::new()
        .route("/health", get(health))
        .route("/api/v1/chat/send", post(send_message))
        .route("/api/v1/chat/rooms", post(create_room))
        .route("/api/v1/chat/history", get(get_history))
        .route("/api/v1/chat/presence", get(get_presence))
        .route("/api/v1/chat/stats", get(get_stats))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .layer(axum::extract::Extension(start))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8127);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("alice-chat-core listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
