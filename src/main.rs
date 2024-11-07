use rmpv::Value;
use socketioxide::{
    extract::{AckSender, Data, SocketRef},
    SocketIo,
};
use tracing::info;
use tracing_subscriber::FmtSubscriber;
use tower_http::services::ServeDir;
use tower_http::cors::CorsLayer;
use http::HeaderValue;
use nanoid::nanoid;

fn on_connect(socket: SocketRef, Data(data): Data<Value>) {
    info!(ns = socket.ns(), ?socket.id, "Socket.IO connected");
    socket.emit("auth", &data).ok();

    socket.on("message", |socket: SocketRef, Data::<Value>(data)| {
        info!(?data, "Received event:");
        socket.emit("message-back", &data).ok();
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
        .layer(cors)
        .layer(layer);

    info!("Starting server");

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}
