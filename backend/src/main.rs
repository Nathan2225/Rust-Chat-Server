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

use rand::{distributions::Alphanumeric, Rng};

use dotenvy::dotenv;
use std::env;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
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
    // existing
    //SetUsername(String),
    Chat(String),
    JoinRoom(String),
    GetRooms, 
    CreateServer(String),
    JoinServer(String),
    GetServers,
    SwitchServer(i32),
    // authentication
    Login { username: String, password: String },
    Signup { username: String, email: String, password: String },
    ResumeSession { token: String },
    Logout,
}


#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum ServerMessage {
    RoomList(Vec<String>),
    ServerList(Vec<(i32, String, String)>),

    // auth
    AuthSuccess { token: String },
    AuthError(String),
}


//client identifier type
type ClientId = usize;


#[derive(Clone)]
struct AuthUser {
    user_id: i32,
    username: String,
}

// * client info and app state * //
#[derive(Clone)]
struct Client {
    user_id: i32,
    username: String,
    sender: mpsc::UnboundedSender<String>,
    server_id: i32, // tracks which server the client is in   
    channel: String,  // room
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

    let mut auth_user: Option<AuthUser> = None;

    // receive messages from client https://websocket.org/guides/languages/rust/
    while let Some(Ok(Message::Text(text))) = ws_receiver.next().await {

        if let Ok(msg) = serde_json::from_str::<ClientMessage>(&text) {


            // blocks everything except Login/Signup from working if not authenticated
            if auth_user.is_none() {
                match &msg {
                    ClientMessage::Login { .. } | ClientMessage::Signup { .. } => {}
                    _ => {
                        let msg = serde_json::to_string(
                            &ServerMessage::AuthError("Please log in first".to_string())
                        ).unwrap();

                        let _ = tx.send(msg);
                        continue;
                    }
                }
            }

            match msg {

                
                // * server logic *
                ClientMessage::CreateServer(server_name) => {
                    if let Some(user) = &auth_user {
                    let username = &user.username;
                    let user_id = user.user_id;
                    let db = state.db.clone();

                    // generate random server code
                    let server_code: String = rand::thread_rng()
                        .sample_iter(&Alphanumeric)
                        .take(8)
                        .map(char::from)
                        .collect();

                    let username_clone = username.clone();
                    let server_name_clone = server_name.clone();

                    // create server and membership
                    let result = tokio::spawn(async move {
                        

                        // insert server
                        let server = sqlx::query!(
                            r#"
                            INSERT INTO servers (name_server, owner_id, server_code)
                            VALUES ($1, $2, $3)
                            RETURNING server_id
                            "#,
                            server_name_clone,
                            user_id,
                            server_code
                        )
                        .fetch_one(&db)
                        .await
                        .ok()?;


                        // create default "general" channel in DB
                        let _ = sqlx::query!(
                            r#"
                            INSERT INTO channels (server_channel, channel_name)
                            VALUES ($1, 'general')
                            ON CONFLICT (server_channel, channel_name) DO NOTHING
                            "#,
                            server.server_id
                        )
                        .execute(&db)
                        .await;


                        // insert into server_members
                        let _ = sqlx::query!(
                            r#"
                            INSERT INTO server_members (user_mem, server_mem)
                            VALUES ($1, $2)
                            "#,
                            user_id,
                            server.server_id
                        )
                        .execute(&db)
                        .await;

                        Some(server.server_id)
                    })
                    .await;

                    // update in memory state
                    if let Ok(Some(server_id)) = result {
                        let mut state = state.inner.lock().await;

                        if let Some(client) = state.clients.get_mut(&client_id) {
                            client.server_id = server_id;
                            client.channel = "general".to_string();
                        }

                        // create default general channel in memory
                        state.servers
                            .entry(server_id.to_string())
                            .or_insert(Server {
                                channels: HashMap::new(),
                            })
                            .channels
                            .entry("general".to_string())
                            .or_insert_with(HashSet::new)
                            .insert(client_id);
                        }

                        // send updated server list to client
                        let sender = {
                            let state_lock = state.inner.lock().await;
                            if let Some(client) = state_lock.clients.get(&client_id) {
                                client.sender.clone()
                            } else {
                                return;
                            }
                        };

                        let db_clone = state.db.clone();
                        let user_id = user_id; // already available above

                        tokio::spawn(async move {
                            let servers = sqlx::query!(
                                r#"
                                SELECT s.server_id, s.name_server, s.server_code
                                FROM servers s
                                JOIN server_members sm ON s.server_id = sm.server_mem
                                WHERE sm.user_mem = $1
                                "#,
                                user_id
                            )
                            .fetch_all(&db_clone)
                            .await;

                            if let Ok(rows) = servers {
                                let server_list: Vec<(i32, String, String)> =
                                    rows.into_iter()
                                        .map(|r| (r.server_id, r.name_server, r.server_code.unwrap_or_default()))
                                        .collect();

                                let msg = serde_json::to_string(
                                    &ServerMessage::ServerList(server_list)
                                ).unwrap();

                                let _ = sender.send(msg);
                            }
                        });

                    }
                }



                ClientMessage::JoinServer(server_code_input) => {
                    if let Some(user) = &auth_user {
                        let user_id = user.user_id;
                        let db = state.db.clone();
                        //let username_clone = username.clone();
                        let code_clone = server_code_input.clone();

                        let result = tokio::spawn(async move {
                            
                            let server = sqlx::query!(
                                "SELECT server_id FROM servers WHERE server_code = $1",
                                code_clone
                            )
                            .fetch_one(&db)
                            .await
                            .ok()?;

                            // insert into server_members avoid duplicates later
                            let _ = sqlx::query!(
                                r#"
                                INSERT INTO server_members (user_mem, server_mem)
                                VALUES ($1, $2)
                                ON CONFLICT DO NOTHING
                                "#,
                                user_id,
                                server.server_id
                            )
                            .execute(&db)
                            .await;

                            Some(server.server_id)
                        })
                        .await;

                        // update in-memory state
                        if let Ok(Some(server_id)) = result {
                            let mut state = state.inner.lock().await;

                            // remove from old channel
                            if let Some(client) = state.clients.get(&client_id) {
                                let old_server_key = client.server_id.to_string();
                                let old_channel = client.channel.clone();

                                if let Some(server) = state.servers.get_mut(&old_server_key) {
                                    if let Some(channel) = server.channels.get_mut(&old_channel) {
                                        channel.remove(&client_id);
                                    }
                                }
                            }

                            // update client
                            if let Some(client) = state.clients.get_mut(&client_id) {
                                client.server_id = server_id;
                                client.channel = "general".to_string();
                            }

                            // ensure server exists in memory
                            state.servers
                                .entry(server_id.to_string())
                                .or_insert(Server {
                                    channels: HashMap::new(),
                                })
                                .channels
                                .entry("general".to_string())
                                .or_insert_with(HashSet::new)
                                .insert(client_id);
                        }

                        // send updated server list
                        let sender = {
                            let state_lock = state.inner.lock().await;
                            if let Some(client) = state_lock.clients.get(&client_id) {
                                client.sender.clone()
                            } else {
                                return;
                            }
                        };

                        let db_clone = state.db.clone();
                        let user_id = user_id; // already available above

                        tokio::spawn(async move {
                            let servers = sqlx::query!(
                                r#"
                                SELECT s.server_id, s.name_server, s.server_code
                                FROM servers s
                                JOIN server_members sm ON s.server_id = sm.server_mem
                                WHERE sm.user_mem = $1
                                "#,
                                user_id
                            )
                            .fetch_all(&db_clone)
                            .await;

                            if let Ok(rows) = servers {
                                let server_list: Vec<(i32, String, String)> =
                                    rows.into_iter()
                                        .map(|r| (r.server_id, r.name_server, r.server_code.unwrap_or_default()))
                                        .collect();

                                let msg = serde_json::to_string(
                                    &ServerMessage::ServerList(server_list)
                                ).unwrap();

                                let _ = sender.send(msg);
                            }
                        });


                    }
                }


                ClientMessage::SwitchServer(new_server_id) => {
                    let db = state.db.clone();
                    let db_clone1 = db.clone();
                    let db_clone2 = db.clone();
                    let mut state = state.inner.lock().await;

                    // get current client
                    let (old_server_key, old_channel, username) =
                        if let Some(client) = state.clients.get(&client_id) {
                            (
                                client.server_id.to_string(),
                                client.channel.clone(),
                                client.username.clone(),
                            )
                        } else {
                            return;
                        };

                    // remove from old server/channel
                    if let Some(server) = state.servers.get_mut(&old_server_key) {
                        if let Some(channel) = server.channels.get_mut(&old_channel) {
                            channel.remove(&client_id);
                        }
                    }

                    // update client to new server
                    if let Some(client) = state.clients.get_mut(&client_id) {
                        client.server_id = new_server_id;
                        client.channel = "general".to_string();
                    }

                    let new_server_key = new_server_id.to_string();

                    // ensure server exists in memory
                    state.servers
                        .entry(new_server_key.clone())
                        .or_insert(Server {
                            channels: HashMap::new(),
                        })
                        .channels
                        .entry("general".to_string())
                        .or_insert_with(HashSet::new)
                        .insert(client_id);

                    // broadcast leave + join
                    broadcast_to_channel(
                        &state,
                        &old_server_key,
                        &old_channel,
                        format!("{} left {}", username, old_channel),
                    );

                    broadcast_to_channel(
                    &state,
                        &new_server_key,
                        "general",
                        format!("{} joined general", username),
                    );

                    // *load channel list*
                    let sender = {
                        if let Some(client) = state.clients.get(&client_id) {
                            client.sender.clone()
                        } else {
                            return;
                        }
                    };

                    let new_server_id_clone = new_server_id;

                    let sender_clone1 = sender.clone();

                    tokio::spawn(async move {
                        let channels = sqlx::query!(
                            r#"
                            SELECT channel_name
                            FROM channels
                            WHERE server_channel = $1
                            "#,
                            new_server_id_clone
                        )
                        .fetch_all(&db_clone1)
                        .await;

                        if let Ok(rows) = channels {
                            let channel_list: Vec<String> =
                                rows.into_iter().map(|r| r.channel_name).collect();

                            let msg = serde_json::to_string(
                                &ServerMessage::RoomList(channel_list)
                            ).unwrap();

                            let _ = sender_clone1.send(msg);
                        }
                    });


                    // *load message history for general channel*
                    //let sender_clone = sender.clone();
                    let sender_clone2 = sender.clone();
                    tokio::spawn(async move {
                        let messages = sqlx::query!(
                            r#"
                            SELECT m.message_content, u.username
                            FROM messages m
                            JOIN users u ON m.user_mes = u.user_id
                            JOIN channels c ON m.channel_mes = c.channel_id
                            WHERE c.channel_name = 'general' AND c.server_channel = $1
                            ORDER BY m.created_at ASC
                            "#,
                            new_server_id
                        )
                        .fetch_all(&db_clone2)
                        .await;

                        if let Ok(messages) = messages {
                            for msg in messages {
                                let formatted = format!(
                                    "{}: {}",
                                    msg.username,
                                    msg.message_content
                                );
                                let _ = sender_clone2.send(formatted);
                            }
                        }
                    });
                }



                // * chat logic *
                ClientMessage::Chat(message) => {
                    if let Some(user) = &auth_user {
                        let name = &user.username;
                        let state_lock = state.inner.lock().await;

                        let client = state_lock.clients.get(&client_id).unwrap();
                        let server_id = client.server_id;
                        let channel = &client.channel;

                        //save message to DB
                        let db = state.db.clone();
                        let message_clone = message.clone();
                        //let username_clone = name.clone();
                        let user_id_clone = user.user_id;
                        let channel_clone = channel.clone();
                        let server_id_clone = server_id;

                        tokio::spawn(async move {
                            let channel_row = sqlx::query!(
                                "SELECT channel_id FROM channels WHERE channel_name = $1 AND server_channel = $2",
                                channel_clone,
                                server_id_clone
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
                                    user_id_clone,
                                    message_clone
                                )
                                .execute(&db)
                                .await;
                            }
                        });

                        let server_name = client.server_id.to_string();
                        
                        broadcast_to_channel(
                            &state_lock,
                            &server_name,
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
                                client.server_id.to_string(),
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
                    let server_name_clone = server_name.clone();

                    let server_id = server_name.parse::<i32>().unwrap();

                    tokio::spawn(async move {
                    let _ = sqlx::query!(
                        r#"
                        INSERT INTO channels (server_channel, channel_name)
                        VALUES ($1, $2)
                        ON CONFLICT (server_channel, channel_name) DO NOTHING
                        "#,
                        server_id,
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
                        format!("{} left {}", username, old_channel.to_string()),
                    );

                    broadcast_to_channel(
                        &state,
                        &server_name,
                        &new_channel,
                        format!("{} joined {}", username, new_channel),
                    );


                    //Load previous messages from DB
                    
                    let channel_name = new_channel.clone();
                    let server_name_clone = server_name.clone();
                    let sender = {
                        if let Some(client) = state.clients.get(&client_id) {
                            client.sender.clone()
                        } else {
                            return;
                        }
                    };

                    let server_id = server_name.parse::<i32>().unwrap();

                    tokio::spawn(async move {
                        let messages = sqlx::query!(
                            r#"
                            SELECT m.message_content, u.username
                            FROM messages m
                            JOIN users u ON m.user_mes = u.user_id
                            JOIN channels c ON m.channel_mes = c.channel_id
                            WHERE c.channel_name = $1 AND c.server_channel = $2
                            ORDER BY m.created_at ASC
                            "#,
                            channel_name,
                            server_id
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


                    let state_lock = state.inner.lock().await;
                    let client = state_lock.clients.get(&client_id).unwrap();
                    let server_id = client.server_id;

                    let server_id_clone = server_id;
                    tokio::spawn(async move {
                        let channels = sqlx::query!(
                            r#"
                            SELECT channel_name
                            FROM channels
                            WHERE server_channel = $1
                            "#,
                            server_id_clone
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


                ClientMessage::GetServers => {
                    let db = state.db.clone();

                    let sender = {
                        let state_lock = state.inner.lock().await;
                        if let Some(client) = state_lock.clients.get(&client_id) {
                            client.sender.clone()
                        } else {
                            return;
                        }
                    };

                    let username_clone = username.clone();

                    tokio::spawn(async move {
                        if let Some(username) = username_clone {
                            // get user_id
                            let user = sqlx::query!(
                                "SELECT user_id FROM users WHERE username = $1",
                                username
                            )
                            .fetch_one(&db)
                            .await;

                            if let Ok(user) = user {
                                // get servers user belongs to
                                let servers = sqlx::query!(
                                    r#"
                                    SELECT s.server_id, s.name_server, s.server_code
                                    FROM servers s
                                    JOIN server_members sm ON s.server_id = sm.server_mem
                                    WHERE sm.user_mem = $1
                                    "#,
                                    user.user_id
                                )
                                .fetch_all(&db)
                                .await;

                                if let Ok(rows) = servers {
                                    let server_list: Vec<(i32, String, String)> =
                                        rows.into_iter()
                                            .map(|r| (r.server_id, r.name_server, r.server_code.unwrap_or_default()))
                                            .collect();

                                    let msg = serde_json::to_string(
                                        &ServerMessage::ServerList(server_list)
                                    ).unwrap();

                                    let _ = sender.send(msg);
                                }
                            }
                        }
                    });
                }

                ClientMessage::Login { username: login_username, password } => {
                    let db = state.db.clone();
                    let sender = tx.clone();

                    // update auth_user in this scope so we can use it after the DB call
                    let auth_user_ptr = &mut auth_user;

                    // sync DB call to get user by username
                    let result = sqlx::query!(
                        "SELECT user_id, username, password_hash FROM users WHERE username = $1",
                        login_username
                    )
                    .fetch_optional(&db)
                    .await;

                    match result {
                        Ok(Some(user)) => {
                                       
                            let parsed_hash = PasswordHash::new(&user.password_hash).unwrap();

                            if Argon2::default()
                                .verify_password(password.as_bytes(), &parsed_hash)
                                .is_ok()
                            {                           
                    

                                auth_user = Some(AuthUser {
                                    user_id: user.user_id,
                                    username: user.username.clone(),
                                });

                                // add client to state
                                let mut state_lock = state.inner.lock().await;

                                // get default server (main)
                                let server_row = sqlx::query!(
                                    "SELECT server_id FROM servers WHERE name_server = 'main'"
                                )
                                .fetch_one(&state.db)
                                .await
                                .unwrap();

                                let client = Client {
                                    user_id: user.user_id,
                                    username: user.username.clone(),
                                    sender: tx.clone(),
                                    server_id: server_row.server_id,
                                    channel: "general".to_string(),
                                };

                                state_lock.clients.insert(client_id, client);

                                // add to channel
                                let server_key = server_row.server_id.to_string();

                                state_lock.servers
                                    .entry(server_key.clone())
                                    .or_insert(Server {
                                        channels: HashMap::new(),
                                    })
                                    .channels
                                    .entry("general".to_string())
                                    .or_insert_with(HashSet::new)
                                    .insert(client_id);

                                let token: String = rand::thread_rng()
                                    .sample_iter(&Alphanumeric)
                                    .take(32)
                                    .map(char::from)
                                    .collect();

                                // store token in DB
                                let _ = sqlx::query!(
                                    "UPDATE users SET session_token = $1 WHERE user_id = $2",
                                    token,
                                    user.user_id
                                )
                                .execute(&state.db)
                                .await;    

                                // send auth success
                                let msg = serde_json::to_string(
                                    &ServerMessage::AuthSuccess { token: token.clone() }
                                ).unwrap();
                                let _ = sender.send(msg);

                                // send server list
                                let servers = sqlx::query!(
                                    r#"
                                    SELECT s.server_id, s.name_server, s.server_code
                                    FROM servers s
                                    INNER JOIN server_members sm ON sm.server_mem = s.server_id
                                    WHERE sm.user_mem = $1
                                    "#,
                                    user.user_id
                                )
                                .fetch_all(&state.db)
                                .await
                                .unwrap_or_default();

                                let server_list: Vec<(i32, String, String)> = servers
                                    .into_iter()
                                    .map(|s| (s.server_id, s.name_server, s.server_code.unwrap_or_default()))
                                    .collect();

                                let msg = serde_json::to_string(
                                    &ServerMessage::ServerList(server_list)
                                ).unwrap();

                                let _ = sender.send(msg);

                            } else {
                                let msg = serde_json::to_string(
                                    &ServerMessage::AuthError("Invalid password".to_string())
                                ).unwrap();

                                let _ = sender.send(msg);
                            }
                        }

                        Ok(None) => {
                            let msg = serde_json::to_string(
                                &ServerMessage::AuthError("User not found".to_string())
                            ).unwrap();

                            let _ = sender.send(msg);
                        }

                        Err(_) => {
                            let msg = serde_json::to_string(
                                &ServerMessage::AuthError("Login failed".to_string())
                            ).unwrap();

                            let _ = sender.send(msg);
                        }
                    }
                }

                ClientMessage::Signup { username: new_username, email, password } => {
                    let db = state.db.clone();
                    let sender = tx.clone();

                    // check if username exists
                    let existing = sqlx::query!(
                        "SELECT user_id FROM users WHERE username = $1",
                        new_username
                    )
                    .fetch_optional(&db)
                    .await;

                    if let Ok(Some(_)) = existing {
                        let msg = serde_json::to_string(
                            &ServerMessage::AuthError("Username already exists".to_string())
                        ).unwrap();

                        let _ = sender.send(msg);
                        return;
                    }

                    let salt = SaltString::generate(&mut OsRng);

                    let argon2 = Argon2::default();

                    let password_hash = argon2
                        .hash_password(password.as_bytes(), &salt)
                        .unwrap()
                        .to_string();

                    // insert new user
                    let inserted = sqlx::query!(
                        r#"
                        INSERT INTO users (username, email, password_hash)
                        VALUES ($1, $2, $3)
                        RETURNING user_id
                        "#,
                        new_username,
                        email,
                        password_hash
                    )
                    .fetch_one(&db)
                    .await;

                    match inserted {
                        Ok(user) => {

                            auth_user = Some(AuthUser {
                                user_id: user.user_id,
                                username: new_username.clone(),
                            });

                            // add client state
                            let mut state_lock = state.inner.lock().await;

                            let server_row = sqlx::query!(
                                "SELECT server_id FROM servers WHERE name_server = 'main'"
                            )
                            .fetch_one(&state.db)
                            .await
                            .unwrap();

                            let client = Client {
                                user_id: user.user_id,
                                username: new_username.clone(), 
                                sender: tx.clone(),
                                server_id: server_row.server_id,
                                channel: "general".to_string(),
                            };

                            state_lock.clients.insert(client_id, client);

                            let server_key = server_row.server_id.to_string();

                            state_lock.servers
                                .entry(server_key.clone())
                                .or_insert(Server {
                                    channels: HashMap::new(),
                                })
                                .channels
                                .entry("general".to_string())
                                .or_insert_with(HashSet::new)
                                .insert(client_id);

                            let token: String = rand::thread_rng()
                                .sample_iter(&Alphanumeric)
                                .take(32)
                                .map(char::from)
                                .collect();

                            // store token in DB
                            let _ = sqlx::query!(
                                "UPDATE users SET session_token = $1 WHERE user_id = $2",
                                token,
                                user.user_id
                            )
                            .execute(&state.db)
                            .await;

                            // send auth success
                            let msg = serde_json::to_string(
                                &ServerMessage::AuthSuccess { token: token.clone() }
                            ).unwrap();
                            let _ = sender.send(msg);

                            // send server list
                            let servers = sqlx::query!(
                                r#"
                                SELECT s.server_id, s.name_server, s.server_code
                                FROM servers s
                                INNER JOIN server_members sm ON sm.server_mem = s.server_id
                                WHERE sm.user_mem = $1
                                "#,
                                user.user_id
                            )
                            .fetch_all(&state.db)
                            .await
                            .unwrap_or_default();

                            let server_list: Vec<(i32, String, String)> = servers
                                .into_iter()
                                .map(|s| (s.server_id, s.name_server, s.server_code.unwrap_or_default()))
                                .collect();

                            let msg = serde_json::to_string(
                                &ServerMessage::ServerList(server_list)
                            ).unwrap();

                            let _ = sender.send(msg);
                        }

                        Err(_) => {
                            let msg = serde_json::to_string(
                                &ServerMessage::AuthError("Signup failed".to_string())
                            ).unwrap();

                            let _ = sender.send(msg);
                        }
                    }
                }


                ClientMessage::ResumeSession { token } => {
                    let db = state.db.clone();

                    let result = sqlx::query!(
                        "SELECT user_id, username FROM users WHERE session_token = $1",
                        token
                    )
                    .fetch_optional(&db)
                    .await;

                    if let Ok(Some(user)) = result {
                        auth_user = Some(AuthUser {
                            user_id: user.user_id,
                            username: user.username.clone(),
                        });

                        let mut state_lock = state.inner.lock().await;

                        let server_row = sqlx::query!(
                            "SELECT server_id FROM servers WHERE name_server = 'main'"
                        )
                        .fetch_one(&state.db)
                        .await
                        .unwrap();

                        let client = Client {
                            user_id: user.user_id,
                            username: user.username.clone(),
                            sender: tx.clone(),
                            server_id: server_row.server_id,
                            channel: "general".to_string(),
                        };

                        state_lock.clients.insert(client_id, client);

                        let msg = serde_json::to_string(
                            &ServerMessage::AuthSuccess { token }
                        ).unwrap();

                        let _ = tx.send(msg);
                    }
                }

                ClientMessage::Logout => {
                    if let Some(user) = &auth_user {
                        let _ = sqlx::query!(
                            "UPDATE users SET session_token = NULL WHERE user_id = $1",
                            user.user_id
                        )
                        .execute(&state.db)
                        .await;
                    }

                    auth_user = None;
                }


            }
        }
    }



    // disconnect cleanup
    if let Some(name) = username {
        let mut state = state.inner.lock().await;

        // remove client from state
        if let Some(client) = state.clients.remove(&client_id) {
            if let Some(server) = state.servers.get_mut(&client.server_id.to_string()) {
                if let Some(channel) = server.channels.get_mut(&client.channel) {
                    channel.remove(&client_id);
                }
            }

            broadcast_to_channel(
                &state,
                &client.server_id.to_string(),
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
