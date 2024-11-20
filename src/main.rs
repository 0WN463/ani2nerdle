use rmpv::Value;
use socketioxide::{
    extract::{AckSender, Data, SocketRef, State},
    SocketIo,
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};
use tracing::info;
use tracing_subscriber::FmtSubscriber;
use tower_http::cors::CorsLayer;
use http::HeaderValue;
use serde::{Deserialize, Serialize};
use nanoid::nanoid;
use rand::seq::SliceRandom; 
use std::env;


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
    Paired,
    Full
}

impl Lobby {
    fn insert(&self, game_id: String, player_id: String) -> LobbyResult {
        let mut lock = self.0.write().unwrap();

        if let Some((_, p2)) = lock.get_mut(&game_id) {
            if p2.is_some() {
                return LobbyResult::Full;
            }

            *p2 = Some(player_id);

            return LobbyResult::Paired;
        } 

        lock.insert(game_id, (player_id, None));

        LobbyResult::New
    }
}

async fn start_game(s: SocketRef) {
    info!("game id {:?}", s.extensions.get::<GameId>());
    if let Some(x) = s.extensions.get::<GameId>() {
        if let Ok(data) = reqwest::get("https://api.jikan.moe/v4/top/anime?type=tv&filter=bypopularity").await {
            if let Ok(json) = data.json::<MALResponse>().await {
                info!("starting game");
                let choosen_anime = json.data.choose(&mut rand::thread_rng());

                if let Some(a) = choosen_anime {
                    s.within(x.0).emit("start game", &a.mal_id).ok();
                }
            }
        }
    }
}

fn on_connect(socket: SocketRef, Data(data): Data<Value>,) {
    info!(ns = socket.ns(), ?socket.id, "Socket.IO connected");
    socket.emit("auth", &data).ok();

    socket.on("join_game", |s: SocketRef, Data::<EventData>(data),  state: State<Lobby>, ack: AckSender| {
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
            LobbyResult::Paired => {
                ack.send("ok_paired").ok();
            }
            LobbyResult::Full => {
                info!("lobby is full");
                ack.send("room is full").ok();
                return;
            }
        }

        let _ = s.join(data.game_id.clone());
        s.to(data.game_id.clone()).emit("player joined", &data).ok();
    });

    socket.on("start game", start_game);

    socket.on("send anime", |s: SocketRef, Data::<i64>(data)| {
        if let Some(x) = s.extensions.get::<GameId>() {
            s.within(x.0).emit("next anime", &data).ok();
        }
    });


    socket.on("message-with-ack", |Data::<Value>(data), ack: AckSender| {
        info!(?data, "Received event");
        ack.send(&("replied: ".to_owned() + data.as_str().unwrap())).ok();
    });
}

async fn create_game() -> String {
    nanoid!()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing::subscriber::set_global_default(FmtSubscriber::default())?;

    let (layer, io) = SocketIo::builder().with_state(Lobby::default()).build_layer();

    io.ns("/", on_connect);

    let cors = CorsLayer::new()
        .allow_origin(env::var("FRONTEND_URL").unwrap_or("".to_string()).parse::<HeaderValue>().unwrap());

    let app = axum::Router::new()
        .route("/game", axum::routing::post(create_game))
        .layer(layer)
        .layer(cors);

    info!("Starting server");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}
