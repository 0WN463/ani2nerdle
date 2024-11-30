use http::{HeaderValue, StatusCode};
use nanoid::nanoid;
use rand::seq::SliceRandom;
use rmpv::Value;
use serde::{Deserialize, Serialize};
use socketioxide::{
    extract::{AckSender, Data, SocketRef, State},
    SocketIo,
};
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tower_http::cors::CorsLayer;
use tracing::info;
use tracing_subscriber::FmtSubscriber;

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(transparent)]
struct PlayerId(String);

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(transparent)]
struct GameId(String);

#[derive(Deserialize, Serialize, Debug)]
struct EventData {
    game_id: String,
    player_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct Anime {
    mal_id: u32,
}

#[derive(Serialize, Deserialize, Debug)]
struct MALResponse {
    data: Vec<Anime>,
}

#[derive(Clone, Default, Debug)]
struct Lobby(Arc<RwLock<HashMap<String, (String, Option<String>)>>>);

enum LobbyResult {
    New,
    Paired(String),
    Full,
}

impl Lobby {
    fn insert(&self, game_id: String, player_id: String) -> LobbyResult {
        let mut lock = self.0.write().unwrap();

        if let Some((p1, p2)) = lock.get_mut(&game_id) {
            if p2.is_some() {
                return LobbyResult::Full;
            }

            *p2 = Some(player_id);

            return LobbyResult::Paired(p1.to_string());
        }

        lock.insert(game_id, (player_id, None));

        LobbyResult::New
    }

    fn remove(&self, game_id: String, player_id: String) {
        let mut lock = self.0.write().unwrap();
        let Some((p1, p2)) = lock.get_mut(&game_id) else {
            return;
        };

        if *p1 == *player_id {
            info!(
                "host left. game ID: {:?}, player ID: {:?}",
                game_id, player_id
            );
            lock.remove(&game_id);
        } else if *p2 == Some(player_id.clone()) {
            info!(
                "guest left. game ID: {:?}, player ID: {:?}",
                game_id, player_id
            );
            *p2 = None;
        } else {
            info!(
                "invalid removal of player. game ID: {:?}, player ID: {:?}",
                game_id, player_id
            );
        }
    }
}

fn timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

async fn start_game(s: SocketRef) {
    info!("game id {:?}", s.extensions.get::<GameId>());
    let Some(x) = s.extensions.get::<GameId>() else {
        return;
    };

    let Ok(data) =
        reqwest::get("https://api.jikan.moe/v4/top/anime?type=tv&filter=bypopularity").await
    else {
        return;
    };

    let Ok(json) = data.json::<MALResponse>().await else {
        return;
    };

    let choosen_anime = json.data.choose(&mut rand::thread_rng());

    let Some(choosen_anime) = choosen_anime else {
        return;
    };

    info!(
        "starting game; anime: {:?}, ts: {}",
        choosen_anime,
        timestamp()
    );
    s.within(x.0)
        .emit("start game", &(choosen_anime.mal_id, timestamp()))
        .ok();
}

async fn on_pass(s: SocketRef) {
    let Some(x) = s.extensions.get::<GameId>() else {
        return;
    };

    s.within(x.0).emit("pass", &timestamp()).ok();
}

fn on_connect(socket: SocketRef, Data(data): Data<Value>) {
    info!(ns = socket.ns(), ?socket.id, "Socket.IO connected");
    socket.emit("auth", &data).ok();

    socket.on(
        "join_game",
        |s: SocketRef, Data::<EventData>(data), state: State<Lobby>, ack: AckSender| {
            if s.extensions.get::<PlayerId>().is_some() {
                return;
            }

            s.extensions.insert(PlayerId(data.player_id.clone()));
            s.extensions.insert(GameId(data.game_id.clone()));

            let res = state.insert(data.game_id.clone(), data.player_id.clone());
            info!("lobby {:?}", state.0);

            match res {
                LobbyResult::New => {
                    ack.send("ok_new").ok();
                }
                LobbyResult::Paired(host_id) => {
                    ack.send(&("ok_paired", host_id)).ok();
                }
                LobbyResult::Full => {
                    info!("lobby is full");
                    ack.send("room is full").ok();
                    return;
                }
            }

            let _ = s.join(data.game_id.clone());
            s.to(data.game_id.clone())
                .emit("player joined", &data.player_id.clone())
                .ok();
        },
    );

    socket.on("start game", start_game);
    socket.on("pass", on_pass);
    socket.on("extend", |s: SocketRef| {
        let Some(x) = s.extensions.get::<GameId>() else {
            return;
        };

        s.within(x.0).emit("extend", &()).ok();
    });

    socket.on("send anime", |s: SocketRef, Data::<i64>(data)| {
        let Some(x) = s.extensions.get::<GameId>() else {
            return;
        };

        s.within(x.0).emit("next anime", &(data, timestamp())).ok();
    });

    socket.on("message-with-ack", |Data::<Value>(data), ack: AckSender| {
        info!(?data, "Received event");
        ack.send(&("replied: ".to_owned() + data.as_str().unwrap()))
            .ok();
    });

    socket.on_disconnect(|s: SocketRef, state: State<Lobby>| {
        let Some(g) = s.extensions.get::<GameId>() else {
            info!("Disconnected with no game ID");
            return;
        };

        let Some(p) = s.extensions.get::<PlayerId>() else {
            info!("Disconnected with no player ID");
            return;
        };

        info!("Disconnected with game ID: {:?}, player ID: {:?}", g, p);
        state.remove(g.0, p.0);
    });
}

async fn create_game() -> String {
    nanoid!()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing::subscriber::set_global_default(FmtSubscriber::default())?;

    let (layer, io) = SocketIo::builder()
        .with_state(Lobby::default())
        .build_layer();

    io.ns("/", on_connect);

    let cors = CorsLayer::new().allow_origin(
        env::var("FRONTEND_URL")
            .unwrap_or("".to_string())
            .parse::<HeaderValue>()
            .unwrap(),
    );

    let app = axum::Router::new()
        .route("/game", axum::routing::post(create_game))
        .route(
            "/healthz",
            axum::routing::get(|| async { StatusCode::NO_CONTENT }),
        )
        .layer(layer)
        .layer(cors);

    info!("Starting server");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}
