# Web Socket Chat Server for advanced programming

## Goal
The goal of this project is to create a real time chat application from scratch using Rust. 
The primary features of this are to allow both text-based messaging by using web sockets, 
and voice/video calling by using Web RTC. By the end, it should work similarly to discord 
supporting real time communications and allowing for chatting and calling between friends 
or for meetings where a camera and microphone are required.


## Technologies
Rust + WebSockets: Rust is a great option for building a real-time chat server-based application. 
While there is a bit of a learning curve, Rust is known for being fast, secure, and reliable. 
To expand on what Rust is capable of, it is fast and memory efficient with no garbage collector, 
uses an ownership model to provide memory safety, and has plenty of documentation and tools. 
Because Rust is great for memory safety and performance, it is the best choice for web socket implementation.
Web sockets are a communication protocol enabling connections between clients and a server over a TCP (transmission control protocol)
connection in real time. Rusts ecosystem provides Web socket libraries such as Tokio runtime for async implementations 
and Actix for web applications. Together, Rust + Web sockets using a library like Tokio is crucial for the development of a 
scalable and real-time chat server.

WebRTC: WebRTC allows for real-time communication capabilities to an application that works on top of an open standard.
By implemnting WebRTC, this project will be able to use camera and microphone capabilities for calling. 
A big challenge involved with this technology is that devices operate behind Network Adress Translation systems and firewalls.
Therefore, STUN (session traversal utilities for NAT) or TURN (traversal using relays around NAT) servers can be used to overcome
networking restrictions. The way it works is Web RTC sends a request to the STUN server which examines the request 
and determines the Public IP address and port. The server sends this information to the client where Web RTC peers exchange
the discovered addresses and establish a direct connection. While A turn server on the other hand is not required, 
it is a good fallback plan if the STUN server fails. The way it works is Web RTC client authenticates the TURN server 
and requests the allocation of resources on the TURN server. The TURN server then provides a public relay address to be shared with peers,
and all traffic is sent to the TURN server which is forwarded to the destination peer. Together, Rust, Web Sockets, and Web RTC
are the best technologies to include for this real-time web chat server application that focuses on both text based and video/audio communications.

## Project planning
This project will be completed through 3 sprints
Sprint 1: Focused on foundations, backend, and WebSocket integration for peer to peer communication.
Sprint 2: Focused on Group chat communications, frontend improvements, user capablities, and deployment using Digital Ocean
Sprint 3: Focused on WebRTC setup and functionality using audio, microphone, and video.

<img width="1026" height="282" alt="image" src="https://github.com/user-attachments/assets/a7bb07fc-e455-4cfe-9242-d3837767cc0d" />
<img width="1031" height="293" alt="image" src="https://github.com/user-attachments/assets/022301d6-93d7-4e84-beed-679dcb3920ec" />
<img width="1008" height="288" alt="image" src="https://github.com/user-attachments/assets/5d43154c-49e9-4aff-9a38-3b6cc4fcf03b" />



