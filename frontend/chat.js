//chat
const socket = new WebSocket("ws://localhost:3000/ws");

let usernameSet = false;

document.getElementById("usernameInput")
.addEventListener("keypress", function(e) {
    if (e.key === "Enter") {
        setUsername();
    }
});

//connect to server
socket.onopen = () => {
    console.log("Connected to server");
};

// receive messages from server
socket.onmessage = (event) => {
    const messages = document.getElementById("messages");

    const li = document.createElement("li");

    const text = event.data;
    const splitIndex = text.indexOf(":");

    if (splitIndex !== -1) {
        const username = text.substring(0, splitIndex + 1);
        const message = text.substring(splitIndex + 1);

        li.innerHTML =
            "<strong>" + username + "</strong>" + message;
    } else {
        li.textContent = text;
    }

    messages.appendChild(li);
    //scrolls messages
    messages.scrollTop = messages.scrollHeight;
};

//ask for username for client
function setUsername() {
    const input = document.getElementById("usernameInput");
    const username = input.value.trim();

    if (username === "") return;

    socket.send(JSON.stringify({
        type: "SetUsername",
        data: username
    }));

    usernameSet = true;

    //hide username input and show chat
    document.getElementById("usernameSection").style.display = "none";
    document.getElementById("chatSection").style.display = "flex";
}

//send message to server
function sendMessage() {
    if (!usernameSet || socket.readyState !== WebSocket.OPEN) return;

    const input = document.getElementById("msg");
    const text = input.value.trim();

    if (text === "") return;

    socket.send(JSON.stringify({
        type: "Chat",
        data: text
    }));

    input.value = "";
}

//enter to send message
document.getElementById("msg")
.addEventListener("keypress", function(e) {
    if (e.key === "Enter") {
        sendMessage();
    }
});