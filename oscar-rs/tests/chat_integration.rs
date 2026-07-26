//! Exercises the trimmed chat-room flow (create, join, occupants, send,
//! receive) against fake ChatNav and Chat servers layered on top of the
//! same fake auth+BOS setup `session_integration.rs` uses — proving the
//! `request_service_redirect`/`FrameSource` plumbing actually drives a real
//! BOS -> ChatNav -> BOS -> Chat sequence of redirects without deadlocking,
//! and that the resulting `ChatRoomSession` parses/sends what's expected.
//! Same caveat as every other integration test here: this proves internal
//! consistency against a fake server, not that a real Open OSCAR Server
//! agrees byte-for-byte.

use oscar_rs::{login, FlapConnection, ServerAddress, Snac, SnacFamily, SnacHeader, Tlv};
use tokio::net::TcpListener;

const ROOM_TLV_NAME: u16 = 0xD3;

fn build_user_info(name: &str, warning: u16) -> Vec<u8> {
    let mut data = vec![name.len() as u8];
    data.extend_from_slice(name.as_bytes());
    data.extend_from_slice(&warning.to_be_bytes());
    data.extend_from_slice(&0u16.to_be_bytes()); // TLV count
    data
}

fn build_room_info_update(exchange: u16, cookie: &str, instance: u16, room_name: &str) -> Vec<u8> {
    let mut body = exchange.to_be_bytes().to_vec();
    body.push(cookie.len() as u8);
    body.extend_from_slice(cookie.as_bytes());
    body.extend_from_slice(&instance.to_be_bytes());
    body.push(0x02); // DetailLevel
    let name_tlv = Tlv::new(ROOM_TLV_NAME, room_name.as_bytes().to_vec());
    body.extend_from_slice(&1u16.to_be_bytes()); // TLV count
    body.extend(name_tlv.encode());
    body
}

/// Reads and discards frames until one whose channel/family/subtype match,
/// mirroring the "ignore unrelated interleaved traffic" tolerance the real
/// client-side loops already have — keeps this fake server robust against
/// ordering details that aren't the point of this test.
async fn read_matching_snac(conn: &mut FlapConnection, family: u16, subtype: u16) -> Snac {
    loop {
        let frame = conn.read_frame().await.unwrap().unwrap();
        if frame.channel != oscar_rs::FlapChannel::Data {
            continue;
        }
        let Some(snac) = Snac::parse(&frame.payload) else { continue };
        if snac.header.family == family && snac.header.subtype == subtype {
            return snac;
        }
    }
}

async fn send_host_online(conn: &mut FlapConnection) {
    conn.read_frame().await.unwrap(); // channel-1 hello carrying the redirect cookie
    let host_online = Snac {
        header: SnacHeader { family: SnacFamily::Generic.as_u16(), subtype: 0x03, flags: 0, request_id: 1 },
        body: Vec::new(),
    };
    conn.send_snac(&host_online).await.unwrap();
}

async fn serve_bos(mut conn: FlapConnection, chatnav_addr: String, chat_addr: String) {
    send_host_online(&mut conn).await;

    // Client announces itself, then immediately requests its (empty, for
    // this test) buddy list — neither is waited on by `login()`, but both
    // land on the wire before the room-create flow starts, so drain them.
    read_matching_snac(&mut conn, SnacFamily::Generic.as_u16(), 0x02).await;
    read_matching_snac(&mut conn, SnacFamily::Feedbag.as_u16(), 0x04).await;

    // First service redirect: ChatNav.
    let request = read_matching_snac(&mut conn, SnacFamily::Generic.as_u16(), 0x04).await;
    let food_group = u16::from_be_bytes([request.body[0], request.body[1]]);
    assert_eq!(food_group, SnacFamily::ChatNav.as_u16());
    let mut reply_body = Vec::new();
    reply_body.extend(Tlv::new(0x05, chatnav_addr.as_bytes().to_vec()).encode());
    reply_body.extend(Tlv::new(0x06, b"chatnav-cookie".to_vec()).encode());
    let reply = Snac {
        header: SnacHeader { family: SnacFamily::Generic.as_u16(), subtype: 0x05, flags: 0, request_id: request.header.request_id },
        body: reply_body,
    };
    conn.send_snac(&reply).await.unwrap();

    // Second service redirect: the specific Chat room.
    let request = read_matching_snac(&mut conn, SnacFamily::Generic.as_u16(), 0x04).await;
    let food_group = u16::from_be_bytes([request.body[0], request.body[1]]);
    assert_eq!(food_group, SnacFamily::Chat.as_u16());
    let mut reply_body = Vec::new();
    reply_body.extend(Tlv::new(0x05, chat_addr.as_bytes().to_vec()).encode());
    reply_body.extend(Tlv::new(0x06, b"chat-cookie".to_vec()).encode());
    let reply = Snac {
        header: SnacHeader { family: SnacFamily::Generic.as_u16(), subtype: 0x05, flags: 0, request_id: request.header.request_id },
        body: reply_body,
    };
    conn.send_snac(&reply).await.unwrap();
}

async fn serve_chat_nav(mut conn: FlapConnection) {
    send_host_online(&mut conn).await;

    let request = read_matching_snac(&mut conn, SnacFamily::ChatNav.as_u16(), 0x08).await;
    // Skip Exchange(2) + Cookie("create": 1+6) + Instance(2) + DetailLevel(1)
    // + the 2-byte TLV count `create_room` writes before the actual TLVs —
    // `Tlv::parse_all` walks tag/len/value pairs with no count of its own,
    // so that count field has to be skipped, not fed in as if it were a TLV.
    let tlvs = Tlv::parse_all(&request.body[14..]);
    let room_name = String::from_utf8_lossy(tlvs.get(&ROOM_TLV_NAME).unwrap()).to_string();
    assert_eq!(room_name, "MyRoom");

    let room_info = build_room_info_update(4, "4-0-MyRoom", 0, &room_name);
    let mut reply_body = Vec::new();
    reply_body.extend(Tlv::new(0x04, room_info).encode());
    let reply = Snac {
        header: SnacHeader { family: SnacFamily::ChatNav.as_u16(), subtype: 0x09, flags: 0, request_id: request.header.request_id },
        body: reply_body,
    };
    conn.send_snac(&reply).await.unwrap();
}

async fn serve_chat_room(mut conn: FlapConnection) {
    send_host_online(&mut conn).await;
    read_matching_snac(&mut conn, SnacFamily::Generic.as_u16(), 0x02).await; // ClientOnline

    // Mandated joiner-only sequence: full occupant list, then room info.
    let mut occupants_body = build_user_info("TestScreenName", 0);
    occupants_body.extend(build_user_info("Friend1", 10));
    let users_joined = Snac {
        header: SnacHeader { family: SnacFamily::Chat.as_u16(), subtype: 0x03, flags: 0, request_id: 1 },
        body: occupants_body,
    };
    conn.send_snac(&users_joined).await.unwrap();

    let room_info_update = Snac {
        header: SnacHeader { family: SnacFamily::Chat.as_u16(), subtype: 0x02, flags: 0, request_id: 2 },
        body: build_room_info_update(4, "4-0-MyRoom", 0, "MyRoom"),
    };
    conn.send_snac(&room_info_update).await.unwrap();

    // Client sends a message to the room.
    let sent = read_matching_snac(&mut conn, SnacFamily::Chat.as_u16(), 0x05).await;
    let tlvs = Tlv::parse_all(&sent.body[10..]); // skip 8-byte cookie + 2-byte channel
    let inner = Tlv::parse_all(tlvs.get(&0x05).unwrap());
    let text = String::from_utf8_lossy(inner.get(&0x01).unwrap()).to_string();
    assert_eq!(text, "hello room");

    // Server pushes a message from someone else in the room.
    let mut incoming_body = vec![0u8; 8]; // message cookie
    incoming_body.extend_from_slice(&1u16.to_be_bytes()); // channel
    incoming_body.extend(Tlv::new(0x03, build_user_info("Friend1", 10)).encode());
    let text_tlv = Tlv::new(0x01, b"hi from friend".to_vec());
    incoming_body.extend(Tlv::new(0x05, text_tlv.encode()).encode());
    let incoming = Snac {
        header: SnacHeader { family: SnacFamily::Chat.as_u16(), subtype: 0x06, flags: 0, request_id: 3 },
        body: incoming_body,
    };
    conn.send_snac(&incoming).await.unwrap();

    // Friend1 leaves.
    let left = Snac {
        header: SnacHeader { family: SnacFamily::Chat.as_u16(), subtype: 0x04, flags: 0, request_id: 4 },
        body: build_user_info("Friend1", 10),
    };
    conn.send_snac(&left).await.unwrap();
}

#[tokio::test]
async fn create_and_join_room_round_trip() {
    let bos_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bos_port = bos_listener.local_addr().unwrap().port();
    let bos_address_str = format!("127.0.0.1:{bos_port}");

    let auth_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let auth_port = auth_listener.local_addr().unwrap().port();

    let chatnav_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let chatnav_port = chatnav_listener.local_addr().unwrap().port();
    let chatnav_address_str = format!("127.0.0.1:{chatnav_port}");

    let chat_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let chat_port = chat_listener.local_addr().unwrap().port();
    let chat_address_str = format!("127.0.0.1:{chat_port}");

    let auth_task = tokio::spawn(async move {
        let (stream, _) = auth_listener.accept().await.unwrap();
        let mut conn = FlapConnection::from_stream(stream);

        conn.read_frame().await.unwrap(); // hello
        conn.read_frame().await.unwrap(); // auth key request

        let auth_key_reply = Snac {
            header: SnacHeader { family: SnacFamily::Authorization.as_u16(), subtype: 0x07, flags: 0, request_id: 1 },
            body: Tlv::new(0x01, b"fake-challenge".to_vec()).encode(),
        };
        conn.send_snac(&auth_key_reply).await.unwrap();

        conn.read_frame().await.unwrap(); // login request

        let mut body = Vec::new();
        body.extend(Tlv::new(0x05, bos_address_str.as_bytes().to_vec()).encode());
        body.extend(Tlv::new(0x06, b"fake-session-cookie".to_vec()).encode());
        let login_reply = Snac {
            header: SnacHeader { family: SnacFamily::Authorization.as_u16(), subtype: 0x03, flags: 0, request_id: 2 },
            body,
        };
        conn.send_snac(&login_reply).await.unwrap();
    });

    let bos_task = tokio::spawn(async move {
        let (stream, _) = bos_listener.accept().await.unwrap();
        let conn = FlapConnection::from_stream(stream);
        serve_bos(conn, chatnav_address_str, chat_address_str).await;
    });

    let chat_nav_task = tokio::spawn(async move {
        let (stream, _) = chatnav_listener.accept().await.unwrap();
        let conn = FlapConnection::from_stream(stream);
        serve_chat_nav(conn).await;
    });

    let chat_task = tokio::spawn(async move {
        let (stream, _) = chat_listener.accept().await.unwrap();
        let conn = FlapConnection::from_stream(stream);
        serve_chat_room(conn).await;
    });

    let server = ServerAddress::parse(&format!("127.0.0.1:{auth_port}")).unwrap();
    let mut session = login(&server, "TestScreenName", "hunter2").await.expect("login should succeed against the fake server");

    let mut frames = session.split_reader();
    let mut room = session.create_and_join_room("MyRoom", &mut frames).await.expect("room creation/join should succeed");

    assert_eq!(room.handle.room_cookie, "4-0-MyRoom");
    assert_eq!(room.handle.room_name, "MyRoom");
    assert_eq!(room.occupants.len(), 2);
    assert!(room.occupants.iter().any(|o| o.screen_name == "TestScreenName"));
    assert!(room.occupants.iter().any(|o| o.screen_name == "Friend1"));

    room.send_message("hello room").await.unwrap();

    // Incoming message from Friend1.
    room.handle_next_frame().await.unwrap();
    assert_eq!(room.messages.len(), 1);
    assert_eq!(room.messages[0].from, "Friend1");
    assert_eq!(room.messages[0].text, "hi from friend");

    // Friend1 leaves.
    room.handle_next_frame().await.unwrap();
    assert_eq!(room.occupants.len(), 1);
    assert_eq!(room.occupants[0].screen_name, "TestScreenName");

    auth_task.await.unwrap();
    bos_task.await.unwrap();
    chat_nav_task.await.unwrap();
    chat_task.await.unwrap();
}
