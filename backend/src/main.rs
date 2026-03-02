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

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use tokio::sync::{mpsc, Mutex};

//allows sending and recieving messages
use futures_util::{SinkExt, StreamExt};

//allows multiple clients to see the same content broadcasted
//use tokio::sync::broadcast;

//tcp listener to bind server to address
use tokio::net::TcpListener;

// for serializing and deserializing JSON messages
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type", content = "data")]
enum ClientMessage {
    SetUsername(String),
    Chat(String),
}

//client identifier type
type ClientId = usize;

// * client info and app state * //
#[derive(Clone)]
struct Client {
    username: String,
    sender: mpsc::UnboundedSender<String>,
    room: String,
}


// * allow clients to send messages and see others *
#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<ServerState>>,
}

struct ServerState {
    //client info
    clients: HashMap<ClientId, Client>,
    //room info
    rooms: HashMap<String, HashSet<ClientId>>,
    //used to assign unique ids to clients
    next_id: ClientId,
}

//confirm running
async fn root() -> &'static str {
    "Chat Server is Running"
}

// handeler for client connection with web socket
//https://docs.rs/axum/latest/axum/extract/ws/
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}




// * web socket logic *
// references used: 
// https://websocket.org/guides/languages/rust/ 
// https://docs.rs/axum/latest/axum/extract/ws/
//https://docs.rs/axum/latest/axum/extract/ws/
//handels communication for a client
async fn handle_socket(stream: WebSocket, state: AppState) {
    println!("Client connected!");

    let (mut ws_sender, mut ws_receiver) = stream.split();

    // channel used to send messages TO this client
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // forwards server messages https://websocket.org/guides/languages/rust/
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // assign client id
    let client_id = {
        let mut state = state.inner.lock().await;
        let id = state.next_id;
        state.next_id += 1;
        id
    };

    // store username
    let mut username: Option<String> = None;

    // receive messages from client https://websocket.org/guides/languages/rust/
    while let Some(Ok(Message::Text(text))) = ws_receiver.next().await {

        if let Ok(msg) = serde_json::from_str::<ClientMessage>(&text) {

            match msg {

                // * username logic *
                ClientMessage::SetUsername(name) => {
                    username = Some(name.clone());

                    let mut state = state.inner.lock().await;

                    let client = Client {
                        username: name.clone(),
                        sender: tx.clone(),
                        room: "general".to_string(),
                    };

                    state.clients.insert(client_id, client);
                    state.rooms
                        .get_mut("general")
                        .unwrap()
                        .insert(client_id);

                    broadcast_to_room(
                        &state,
                        "general",
                        format!("{} joined", name),
                    );
                }

                // * chat logic *
                ClientMessage::Chat(message) => {
                    if let Some(name) = &username {
                        let state = state.inner.lock().await;

                        let client = state.clients.get(&client_id).unwrap();
                        let room = &client.room;

                        broadcast_to_room(
                            &state,
                            room,
                            format!("{}: {}", name, message),
                        );
                    }
                }
            }
        }
    }



    // disconnect cleanup
    if let Some(name) = username {
        let mut state = state.inner.lock().await;

        // remove client from state
        if let Some(client) = state.clients.remove(&client_id) {
            if let Some(room) = state.rooms.get_mut(&client.room) {
                room.remove(&client_id);
            }

            broadcast_to_room(
                &state,
                &client.room,
                format!("{} left", name),
            );
        }
    }

    println!("Client disconnected");
}




// helper to send a message to all clients in a room
fn broadcast_to_room(
    state: &ServerState,
    room: &str,
    message: String,
) {
    if let Some(clients) = state.rooms.get(room) {
        for client_id in clients {
            if let Some(client) = state.clients.get(client_id) {
                let _ = client.sender.send(message.clone());
            }
        }
    }
}



// main async tokio
#[tokio::main]
async fn main() {
    //for logging and tracing output
    tracing_subscriber::fmt::init();


    // stored as shared application state
    let state = AppState {
        inner: Arc::new(Mutex::new(ServerState {
            clients: HashMap::new(),
            rooms: {
                let mut r = HashMap::new();
                r.insert("general".to_string(), HashSet::new());
                r
            },
            next_id: 0,
        })),
    };

    //set router
    let app = Router::new()
        .route("/", get(root))
        .route("/ws", get(ws_handler))
        .with_state(state);

    //server's address
    let addr = "127.0.0.1:3000";

    println!("Server running at http://{}", addr);

    //TCP listener for address
    let listener = TcpListener::bind(addr)
        .await
        .unwrap();

    //start server
    axum::serve(listener, app)
        .await
        .unwrap();
}
