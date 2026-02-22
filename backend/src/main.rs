// axum for routing, requests, and WebSocket support.
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
//allows sending and recieving messages
use futures_util::{SinkExt, StreamExt};
//allows multiple clients to see the same content broadcasted
use tokio::sync::broadcast;
//tcp listener to bind server to address
use tokio::net::TcpListener;


// * allow clients to send messages and see others *
#[derive(Clone)]
struct AppState {
    tx: broadcast::Sender<String>,
}

// confirm running
async fn root() -> &'static str {
    "Rust Chat Server is Running"
}

// handeler for client connection with web socket
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

// * web socket logic *
//handels communication for a client
async fn handle_socket(stream: WebSocket, state: AppState) {
    println!("Client connected!");

    //splits web socket into sender and receiver
    let (mut sender, mut receiver) = stream.split();

    // subscribes client to broadcast channel
    let mut rx = state.tx.subscribe();

    //send messages to client
    //forwards messages from channel to web socket client
    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender
                .send(Message::Text(msg.into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    // receive messages from client
    // read websocket messages and broadcast them
    let tx = state.tx.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(text))) = receiver.next().await {
            println!("Received: {}", text);

            // convert Utf 8 Bytes to String and send
            let _ = tx.send(text.to_string());
        }
    });

    // wait for sending or receiving to stop
    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    }
    //print when ending
    println!("Client disconnected");
}

// main async tokio
#[tokio::main]
async fn main() {
    //for logging and tracing output
    tracing_subscriber::fmt::init();

    //broadcast channel
    //send messages with capacity of 100
    let (tx, _) = broadcast::channel(100);

    // stored as shared application state
    let state = AppState { tx };

    //set router
    let app = Router::new()
        .route("/", get(root))
        .route("/ws", get(ws_handler))
        .with_state(state);

    //server's address
    let addr = "127.0.0.1:3000";

    println!("Server running at http://{}", addr);

    //TCP listener fro address
    let listener = TcpListener::bind(addr)
        .await
        .unwrap();

    //start server
    axum::serve(listener, app)
        .await
        .unwrap();
}
