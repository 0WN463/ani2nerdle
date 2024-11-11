use rmpv::Value;
use socketioxide::{
    extract::{AckSender, Data, SocketRef},
    SocketIo,
};
use tracing::info;
use tracing_subscriber::FmtSubscriber;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::cors::CorsLayer;
use http::HeaderValue;
use serde::{Deserialize, Serialize};
use nanoid::nanoid;
use rand::seq::SliceRandom; 


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

async fn start_game(s: SocketRef) {
    info!("game id {:?}", s.extensions.get::<GameId>());
    match s.extensions.get::<GameId>() {
        Some(x) => {

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
        None => {
            return 
        }
    }
}

fn on_connect(socket: SocketRef, Data(data): Data<Value>) {
    info!(ns = socket.ns(), ?socket.id, "Socket.IO connected");
    socket.emit("auth", &data).ok();

    socket.on("join_game", |s: SocketRef, Data::<EventData>(data)| {
        if s.extensions.get::<PlayerId>().is_some() {
            return;
        }

        s.extensions.insert(PlayerId(data.player_id.clone()));
        s.extensions.insert(GameId(data.game_id.clone()));
        let _ = s.join(data.game_id.clone());

        s.to(data.game_id.clone()).emit("player joined", &data).ok();
    });

    socket.on("start game", start_game);

    socket.on("send anime", |s: SocketRef, Data::<i64>(data)| {
        match s.extensions.get::<GameId>() {
            Some(x) => {
                s.within(x.0).emit("next anime", &data).ok();
            }
            None => {
                return
            }
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

    let (layer, io) = SocketIo::new_layer();

    io.ns("/", on_connect);
    io.ns("/custom", on_connect);

    let cors = CorsLayer::new()
        .allow_origin( "http://localhost:3001".parse::<HeaderValue>().unwrap());

    let app = axum::Router::new()
        .nest_service("/", ServeDir::new("build"))
        .route("/game", axum::routing::post(create_game))
        .route_service("/game/:game_id", ServeFile::new("build/index.html"))
        .layer(cors)
        .layer(layer);

    info!("Starting server");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}
