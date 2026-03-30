

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

use dotenvy::dotenv;
use std::env;

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
    JoinRoom(String),
    GetRooms, // NEW
}


#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum ServerMessage {
    RoomList(Vec<String>),
}


//client identifier type
type ClientId = usize;

// * client info and app state * //
#[derive(Clone)]
struct Client {
    username: String,
    sender: mpsc::UnboundedSender<String>,
    server: String,   // NEW
    channel: String,  // renamed from room
}


// * allow clients to send messages and see others *
use sqlx::PgPool;

#[derive(Clone)]
struct AppState {
    inner: Arc<Mutex<ServerState>>,
    db: PgPool,
}

struct ServerState {
    //client info
    clients: HashMap<ClientId, Client>,
    //room info
    servers: HashMap<String, Server>,
    //used to assign unique ids to clients
    next_id: ClientId,
}

struct Server {
    channels: HashMap<String, HashSet<ClientId>>,
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

                    // insert user into DB
                    let db = state.db.clone();
                    let name_clone = name.clone();

                    tokio::spawn(async move {
                    let _ = sqlx::query!(
                            "INSERT INTO users (username, password_hash)
                            VALUES ($1, 'temp')
                            ON CONFLICT (username) DO NOTHING",
                            name_clone
                        )
                        .execute(&db)
                        .await;
                    });

                    let mut state = state.inner.lock().await;

                    let client = Client {
                    username: name.clone(),
                        sender: tx.clone(),
                        server: "main".to_string(),
                        channel: "general".to_string(),
                    };

                    state.clients.insert(client_id, client);

                    // add to default server/channel
                    if let Some(server) = state.servers.get_mut("main") {
                        server
                            .channels
                            .entry("general".to_string())
                            .or_insert_with(HashSet::new)
                            .insert(client_id);
                    }

                    broadcast_to_channel(
                        &state,
                        "main",
                        "general",
                        format!("{} joined", name),
                    );

                    broadcast_room_list(&state);
                }

                // * chat logic *
                ClientMessage::Chat(message) => {
                    if let Some(name) = &username {
                        let state_lock = state.inner.lock().await;

                        let client = state_lock.clients.get(&client_id).unwrap();
                        let server_name = &client.server;
                        let channel = &client.channel;

                        //save message to DB
                        let db = state.db.clone();
                        let message_clone = message.clone();
                        let username_clone = name.clone();
                        let channel_clone = channel.clone();

                        tokio::spawn(async move {
                            // get user_id
                            let user = sqlx::query!(
                                "SELECT user_id FROM users WHERE username = $1",
                            username_clone
                            )
                            .fetch_optional(&db)
                            .await
                            .ok()
                            .flatten();

                            if let Some(user) = user {
                                // get channel_id
                                let channel_row = sqlx::query!(
                                "SELECT channel_id FROM channels WHERE channel_name = $1",
                                    channel_clone
                                )
                                .fetch_optional(&db)
                                .await
                                .ok()
                                .flatten();

                                if let Some(channel_row) = channel_row {
                                    let _ = sqlx::query!(
                                        "INSERT INTO messages (channel_mes, user_mes, message_content)
                                        VALUES ($1, $2, $3)",
                                        channel_row.channel_id,
                                        user.user_id,
                                        message_clone
                                    )
                                .execute(&db)
                                    .await;
                                }
                            }
                        });

                        broadcast_to_channel(
                            &state_lock,
                            server_name,
                            channel,
                            format!("{}: {}", name, message),
                        );
                    }
                }


                // * room logic *
                ClientMessage::JoinRoom(new_channel) => {
                    let db = state.db.clone();
                    let mut state = state.inner.lock().await;

                    let (server_name, old_channel, username) =
                        if let Some(client) = state.clients.get(&client_id) {
                            (
                                client.server.clone(),
                                client.channel.clone(),
                                client.username.clone(),
                            )
                        } else {
                            return;
                        };

                    // remove from old channel
                    if let Some(server) = state.servers.get_mut(&server_name) {
                        if let Some(channel) = server.channels.get_mut(&old_channel) {
                            channel.remove(&client_id);
                        }
                    }


                    //insert channel into DB if not exists
                    let db_clone = db.clone();
                    let channel_name_clone = new_channel.clone();

                    tokio::spawn(async move {
                        let _ = sqlx::query!(
                            r#"
                            INSERT INTO channels (server_channel, channel_name)
                            VALUES (
                                (SELECT server_id FROM servers WHERE name_server = 'main'),
                                $1
                            )
                            ON CONFLICT (server_channel, channel_name) DO NOTHING
                            "#,
                            channel_name_clone
                        )
                        .execute(&db_clone)
                        .await;
                    });



                    // add to new channel
                    if let Some(server) = state.servers.get_mut(&server_name) {
                        server
                            .channels
                            .entry(new_channel.clone())
                            .or_insert_with(HashSet::new)
                            .insert(client_id);
                    }

                    // update client
                    if let Some(client) = state.clients.get_mut(&client_id) {
                        client.channel = new_channel.clone();
                    }

                    // broadcast
                    broadcast_to_channel(
                        &state,
                        &server_name,
                        &old_channel,
                        format!("{} left {}", username, old_channel),
                    );

                    broadcast_to_channel(
                        &state,
                        &server_name,
                        &new_channel,
                        format!("{} joined {}", username, new_channel),
                    );


                    //Load previous messages from DB
                    
                    let channel_name = new_channel.clone();
                    let sender = {
                        if let Some(client) = state.clients.get(&client_id) {
                            client.sender.clone()
                        } else {
                            return;
                        }
                    };

                    tokio::spawn(async move {
                        let messages = sqlx::query!(
                            r#"
                            SELECT m.message_content, u.username
                            FROM messages m
                            JOIN users u ON m.user_mes = u.user_id
                            JOIN channels c ON m.channel_mes = c.channel_id
                            WHERE c.channel_name = $1
                            ORDER BY m.created_at ASC
                            "#,
                            channel_name
                        )
                        .fetch_all(&db)
                        .await;

                        if let Ok(messages) = messages {
                            for msg in messages {
                                let formatted = format!(
                                    "{}: {}",
                                    msg.username,
                                    msg.message_content
                                );
                                let _ = sender.send(formatted);
                            }
                        }
                    });
                }


                // * room list logic *
                ClientMessage::GetRooms => {
                    let db = state.db.clone();

                    let sender = {
                        let state = state.inner.lock().await;
                        if let Some(client) = state.clients.get(&client_id) {
                            client.sender.clone()
                        } else {
                            return;
                        }
                    };

                    tokio::spawn(async move {
                        let channels = sqlx::query!(
                            r#"
                            SELECT channel_name
                            FROM channels
                            WHERE server_channel = (
                                SELECT server_id FROM servers WHERE name_server = 'main'
                            )
                            "#
                        )
                        .fetch_all(&db)
                        .await;

                        if let Ok(rows) = channels {
                            let channel_list: Vec<String> =
                                rows.into_iter().map(|r| r.channel_name).collect();

                            let msg = serde_json::to_string(
                                &ServerMessage::RoomList(channel_list)
                            ).unwrap();

                            let _ = sender.send(msg);
                        }
                    });
                }

            }
        }
    }



    // disconnect cleanup
    if let Some(name) = username {
        let mut state = state.inner.lock().await;

        // remove client from state
        if let Some(client) = state.clients.remove(&client_id) {
            if let Some(server) = state.servers.get_mut(&client.server) {
                if let Some(channel) = server.channels.get_mut(&client.channel) {
                    channel.remove(&client_id);
                }
            }

            broadcast_to_channel(
                &state,
                &client.server,
                &client.channel,
                format!("{} left", name),
            );
            
            broadcast_room_list(&state);
        }
    }

    println!("Client disconnected");
}




// helper to send a message to all clients in a room
fn broadcast_to_channel(
    state: &ServerState,
    server_name: &str,
    channel: &str,
    message: String,
) {
    if let Some(server) = state.servers.get(server_name) {
        if let Some(clients) = server.channels.get(channel) {
            for client_id in clients {
                if let Some(client) = state.clients.get(client_id) {
                    let _ = client.sender.send(message.clone());
                }
            }
        }
    }
}


// helper to send updated room list to all clients
fn broadcast_room_list(state: &ServerState) {
    let channels: Vec<String> = if let Some(server) = state.servers.get("main") {
        server.channels.keys().cloned().collect()
    } else {
        vec![]
    };

    let msg = serde_json::to_string(
        &ServerMessage::RoomList(channels)
    ).unwrap();

    for client in state.clients.values() {
        let _ = client.sender.send(msg.clone());
    }
}


// main async tokio
#[tokio::main]
async fn main() {
    //for logging and tracing output
    tracing_subscriber::fmt::init();


    dotenv().ok();

    let database_url = env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");

    let db = PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to DB");


    // stored as shared application state
    let state = AppState {
        db,
        inner: Arc::new(Mutex::new(ServerState {
            clients: HashMap::new(),
            servers: {
                let mut servers = HashMap::new();

                let mut main_server = Server {
                    channels: HashMap::new(),
                };

                main_server
                    .channels
                    .insert("general".to_string(), HashSet::new());

                servers.insert("main".to_string(), main_server);

                servers
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
