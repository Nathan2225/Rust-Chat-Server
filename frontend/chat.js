//chat
const isLocal =
    window.location.hostname === "localhost" ||
    window.location.hostname === "127.0.0.1";

const socket = new WebSocket(
    isLocal
        ? "ws://localhost:3000/ws"
        : "wss://thiscord-chat.onrender.com/ws"
        
);

let isAuthenticated = false;
let currentChannel = "general";
let currentServerId = null;
let serverMap = {}; 

socket.onopen = () => {
    console.log("Connected to server");

    const token = localStorage.getItem("token");

    if (token) {
        socket.send(JSON.stringify({
            type: "ResumeSession",
            data: { token }
        }));
    }
};



document.getElementById("roomInput")
.addEventListener("keypress", function(e) {
    if (e.key === "Enter") {
        joinRoom();
    }
});

document.getElementById("msg")
.addEventListener("keypress", function(e) {
    if (e.key === "Enter") {
        sendMessage();
    }
});

// connect to server
socket.onmessage = (event) => {

    // parse JSON
    try {
        const data = JSON.parse(event.data);

        if (data.type === "AuthSuccess") {
            isAuthenticated = true;

            localStorage.setItem("token", data.data.token);

            document.getElementById("authSection").style.display = "none";
            document.getElementById("chatSection").style.display = "flex";

            socket.send(JSON.stringify({ type: "GetServers" }));
            socket.send(JSON.stringify({ type: "GetRooms" }));

            return;
        }

        if (data.type === "AuthError") {
            alert(data.data);
            return;
        }

        // handle room list updates
        if (data.type === "RoomList") {
            updateRoomList(data.data);
            return;
        }

        if (data.type === "ServerList") {
            updateServerList(data.data);
            return;
        }

        

    } catch (e) {
        // not JSON, treat as normal chat message
    }

    // *** normal chat message logic ***
    const messages = document.getElementById("messages");
    const li = document.createElement("li");

    const text = event.data;
    const splitIndex = text.indexOf(":");

    if (splitIndex !== -1) {

        const username = text.substring(0, splitIndex + 1);
        const message = text.substring(splitIndex + 1);

        const strong = document.createElement("strong");
        strong.textContent = username;

        const msgText = document.createTextNode(message);

        li.appendChild(strong);
        li.appendChild(msgText);

    } else {
        li.textContent = text;
    }

    messages.appendChild(li);
    messages.scrollTop = messages.scrollHeight;
};


function login() {
    const username = document.getElementById("loginUsername").value.trim();
    const password = document.getElementById("loginPassword").value.trim();

    if (!username || !password) return;

    socket.send(JSON.stringify({
        type: "Login",
        data: { username, password }
    }));
}

function signup() {
    const username = document.getElementById("signupUsername").value.trim();
    const email = document.getElementById("signupEmail").value.trim();
    const password = document.getElementById("signupPassword").value.trim();

    if (!username || !email || !password) return;

    socket.send(JSON.stringify({
        type: "Signup",
        data: { username, email, password }
    }));
}


function logout() {
    socket.send(JSON.stringify({ type: "Logout" }));

    // clear token
    localStorage.removeItem("token");

    // reset auth state
    isAuthenticated = false;

    // reset client state
    currentChannel = "general";
    currentServerId = null;

    // clear ui lists
    document.getElementById("messages").innerHTML = "";
    document.getElementById("roomList").innerHTML = "";
    document.getElementById("serverList").innerHTML = "";

    // reset current labels
    document.getElementById("currentRoom").textContent = "";
    document.getElementById("currentServer").textContent = "";
    document.getElementById("currentServerCode").textContent = "";

    // clear input fields
    document.getElementById("loginUsername").value = "";
    document.getElementById("loginPassword").value = "";
    document.getElementById("signupUsername").value = "";
    document.getElementById("signupEmail").value = "";
    document.getElementById("signupPassword").value = "";

    document.getElementById("msg").value = "";
    document.getElementById("roomInput").value = "";
    document.getElementById("serverNameInput").value = "";
    document.getElementById("serverCodeInput").value = "";

    // toggle UI
    document.getElementById("authSection").style.display = "block";
    document.getElementById("chatSection").style.display = "none";
}


// join room
function joinRoom() {

    if (!isAuthenticated) return;

    const input = document.getElementById("roomInput");
    const channel = input.value.trim();

    if (channel === "") return;

    socket.send(JSON.stringify({
        type: "JoinRoom", 
        data: channel
    }));

    currentChannel = channel;

    document.getElementById("currentRoom").textContent = channel;

    document.getElementById("messages").innerHTML = "";

    input.value = "";

    setTimeout(() => {
        socket.send(JSON.stringify({ type: "GetRooms" }));
    }, 200);
}

// send chat message
function sendMessage() {

    if (!isAuthenticated || socket.readyState !== WebSocket.OPEN) return;

    const input = document.getElementById("msg");
    const text = input.value.trim();

    if (text === "") return;

    socket.send(JSON.stringify({
        type: "Chat",
        data: text
    }));

    input.value = "";
}

// update room list in sidebar
function updateRoomList(channels) {

    const list = document.getElementById("roomList");

    if (!list) return; 

    list.innerHTML = "";

    channels.forEach(channel => {
        const li = document.createElement("li");

        li.textContent = "# " + channel;

        if (channel === currentChannel) {
            li.style.backgroundColor = "#40444b";
        }

        li.onclick = () => {
            joinRoomFromSidebar(channel);
        };

        list.appendChild(li);
    });
}

// join room when clicking from sidebar
function joinRoomFromSidebar(channelName) {

    if (!isAuthenticated) return;

    socket.send(JSON.stringify({
        type: "JoinRoom",
        data: channelName
    }));

    currentChannel = channelName;

    document.getElementById("currentRoom").textContent = channelName;

    document.getElementById("messages").innerHTML = "";
}


// create server
window.createServer = function() {
    console.log("Create server clicked");

    const input = document.getElementById("serverNameInput");
    const name = input.value.trim();

    if (name === "") return;

    socket.send(JSON.stringify({
        type: "CreateServer",
        data: name
    }));

    input.value = "";

    // request updated server list after a short delay
    setTimeout(() => {
        socket.send(JSON.stringify({ type: "GetServers" }));
    }, 200);
}

// join server by code
window.joinServer = function() {
    console.log("Join server clicked");

    const input = document.getElementById("serverCodeInput");
    const code = input.value.trim();

    if (code === "") return;

    socket.send(JSON.stringify({
        type: "JoinServer",
        data: code
    }));

    input.value = "";

    
    setTimeout(() => {
        socket.send(JSON.stringify({ type: "GetServers" }));
    }, 200);
}



// update server list in sidebar
function updateServerList(servers) {

    const list = document.getElementById("serverList");

    if (!list) return;

    list.innerHTML = "";

    serverMap = {}; // reset

    // servers now expected as [id, name, code]
    servers.forEach(([id, name, code]) => {
        serverMap[id] = { name, code };
    });

    if (servers.length > 0 && currentServerId === null) {
        currentServerId = servers[0][0];

        updateHeaderServer(); 

        socket.send(JSON.stringify({
            type: "SwitchServer",
            data: currentServerId
        }));
    }

    servers.forEach(([id, name]) => {
        const li = document.createElement("li");

        li.textContent = name;

        if (id === currentServerId) {
            li.style.backgroundColor = "#5865f2";
        }

        li.onclick = () => {
            switchServer(id);
        };

        list.appendChild(li);
    });
}

function switchServer(serverId) {

    if (!isAuthenticated) return;

    socket.send(JSON.stringify({
        type: "SwitchServer",
        data: serverId
    }));

    currentServerId = serverId;

    updateHeaderServer(); 
    
    setTimeout(() => {
        socket.send(JSON.stringify({ type: "GetRooms" }));
    }, 100);

    document.getElementById("messages").innerHTML = "";

    currentChannel = "general";
    document.getElementById("currentRoom").textContent = "general";

    socket.send(JSON.stringify({
        type: "GetRooms"
    }));
}

function updateHeaderServer() {

    const serverNameEl = document.getElementById("currentServer");
    const serverCodeEl = document.getElementById("currentServerCode");

    if (!serverMap[currentServerId]) return;

    serverNameEl.textContent = serverMap[currentServerId].name;

    if (serverCodeEl) {
        serverCodeEl.textContent = "invite code: " + (serverMap[currentServerId].code || "N/A");
    }
}