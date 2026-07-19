//! Exercises the ported feedbag (buddy list), presence, Locate (away
//! status), and ICBM (messaging) code against a fake BOS server, the same
//! "actually verified, not just compiled" bar `login_integration.rs` set for
//! the login flow. Byte layouts are still best-effort against real-server
//! behavior (same caveat as everywhere else in this project) — this proves
//! the client-side state machine reacts to server pushes and builds
//! well-formed requests, not that a real Open OSCAR Server agrees with the
//! exact TLV numbers used.

use oscar_rs::{login, Buddy, FeedbagItem, FlapConnection, ServerAddress, Snac, SnacFamily, SnacHeader, Tlv};
use tokio::net::TcpListener;

/// Runs the full fake-BOS script: host-online, then a round of feedbag
/// sync, presence, an incoming IM, and away-status exchange, matching the
/// sequence the test below drives from the client side.
async fn serve_bos_session(mut conn: FlapConnection) {
    // Client hello carrying the auth cookie — ignore content, just consume it.
    conn.read_frame().await.unwrap();

    // "Host online" — client is now considered logged in and will immediately
    // request its buddy list.
    let host_online = Snac {
        header: SnacHeader { family: SnacFamily::Generic.as_u16(), subtype: 0x03, flags: 0, request_id: 1 },
        body: Vec::new(),
    };
    conn.send_snac(&host_online).await.unwrap();

    // Client requests the buddy list (Feedbag family, subtype 0x04).
    let request = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&request.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Feedbag.as_u16());
    assert_eq!(snac.header.subtype, 0x04);

    // Reply with a one-group, one-buddy roster.
    let group = FeedbagItem { name: "Buddies".to_string(), group_id: 0, item_id: 1, class_id: FeedbagItem::CLASS_GROUP, attributes: Vec::new() };
    let buddy = FeedbagItem { name: "Buddy1".to_string(), group_id: 1, item_id: 2, class_id: FeedbagItem::CLASS_BUDDY, attributes: Vec::new() };
    let mut body = vec![0x01, 0x00, 0x02]; // version, item count = 2
    body.extend(group.encode());
    body.extend(buddy.encode());
    body.extend_from_slice(&[0, 0, 0, 0]); // trailing last-modified timestamp, ignored by the client
    let feedbag_reply = Snac {
        header: SnacHeader { family: SnacFamily::Feedbag.as_u16(), subtype: 0x05, flags: 0, request_id: 2 },
        body,
    };
    conn.send_snac(&feedbag_reply).await.unwrap();

    // Client acks receipt (subtype 0x06 "use") once it's processed the reply.
    let ack = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&ack.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Feedbag.as_u16());
    assert_eq!(snac.header.subtype, 0x06);

    // Presence: Buddy1 arrives online, away (status-flags TLV 0x0C, bit
    // 0x0020), and already carrying a warning level (TLV 0x0A, 25.0%).
    let mut arrival_body = vec![6u8];
    arrival_body.extend_from_slice(b"Buddy1");
    arrival_body.extend(Tlv::new(0x0C, 0x0020u16.to_be_bytes().to_vec()).encode());
    arrival_body.extend(Tlv::new(0x0A, 250u16.to_be_bytes().to_vec()).encode());
    let arrival = Snac {
        header: SnacHeader { family: SnacFamily::BuddyPresence.as_u16(), subtype: 0x0B, flags: 0, request_id: 3 },
        body: arrival_body,
    };
    conn.send_snac(&arrival).await.unwrap();

    // Incoming IM from Buddy1.
    let mut im_body = vec![0u8; 8]; // cookie
    im_body.extend_from_slice(&1u16.to_be_bytes()); // channel
    im_body.push(6);
    im_body.extend_from_slice(b"Buddy1");
    let mut message_inner = Vec::new();
    let mut text_fragment = vec![0x00, 0x00];
    text_fragment.extend_from_slice(b"hey there");
    message_inner.extend(Tlv::new(0x0101, text_fragment).encode());
    im_body.extend(Tlv::new(0x02, message_inner).encode());
    let incoming_im = Snac {
        header: SnacHeader { family: SnacFamily::Messaging.as_u16(), subtype: 0x07, flags: 0, request_id: 4 },
        body: im_body,
    };
    conn.send_snac(&incoming_im).await.unwrap();

    // Client replies with its own IM to Buddy1.
    let outgoing = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&outgoing.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Messaging.as_u16());
    assert_eq!(snac.header.subtype, 0x06);
    // Same BUF-then-TLVs layout as the incoming IM: 8-byte cookie, 2-byte
    // channel, then a length-prefixed recipient name, then TLVs.
    let recipient_len = snac.body[10] as usize;
    let recipient = String::from_utf8_lossy(&snac.body[11..11 + recipient_len]).to_string();
    assert_eq!(recipient, "Buddy1");
    let tlvs = Tlv::parse_all(&snac.body[11 + recipient_len..]);
    let message_tlv = tlvs.get(&0x02).unwrap();
    let fragments = Tlv::parse_all(message_tlv);
    let text = String::from_utf8_lossy(&fragments.get(&0x0101).unwrap()[2..]).to_string();
    assert_eq!(text, "hi back");

    // Client sets an away message (Locate family, subtype 0x04 "set info").
    let set_info = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&set_info.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Locate.as_u16());
    assert_eq!(snac.header.subtype, 0x04);
    let tlvs = Tlv::parse_all(&snac.body);
    assert_eq!(String::from_utf8_lossy(tlvs.get(&0x04).unwrap()), "brb");

    // Client asks for Buddy1's info (subtype 0x05 "user info query").
    let query = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&query.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Locate.as_u16());
    assert_eq!(snac.header.subtype, 0x05);
    let tlvs = Tlv::parse_all(&snac.body);
    assert_eq!(String::from_utf8_lossy(tlvs.get(&0x01).unwrap()), "Buddy1");

    // Reply with Buddy1's away message.
    let mut reply_body = vec![6u8];
    reply_body.extend_from_slice(b"Buddy1");
    reply_body.extend(Tlv::new(0x04, b"On a call".to_vec()).encode());
    let user_info_reply = Snac {
        header: SnacHeader { family: SnacFamily::Locate.as_u16(), subtype: 0x06, flags: 0, request_id: 5 },
        body: reply_body,
    };
    conn.send_snac(&user_info_reply).await.unwrap();

    // Client sends a warning to Buddy1 (Messaging family, subtype 0x08).
    let warning_request = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&warning_request.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Messaging.as_u16());
    assert_eq!(snac.header.subtype, 0x08);
    assert_eq!(u16::from_be_bytes([snac.body[0], snac.body[1]]), 0); // not anonymous
    let name_len = snac.body[2] as usize;
    assert_eq!(String::from_utf8_lossy(&snac.body[3..3 + name_len]), "Buddy1");

    // Reply with Buddy1's new warning level (subtype 0x09), echoing the
    // request_id the client used so it can attribute this back to Buddy1.
    let mut warning_reply_body = 0u16.to_be_bytes().to_vec(); // old level
    warning_reply_body.extend_from_slice(&500u16.to_be_bytes()); // new level, 50.0%
    let warning_reply = Snac {
        header: SnacHeader { family: SnacFamily::Messaging.as_u16(), subtype: 0x09, flags: 0, request_id: snac.header.request_id },
        body: warning_reply_body,
    };
    conn.send_snac(&warning_reply).await.unwrap();

    // Client adds Buddy1 to the block list (Feedbag family, subtype 0x08
    // "insert item", class_id 0x0003 "deny").
    let block_request = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&block_request.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Feedbag.as_u16());
    assert_eq!(snac.header.subtype, 0x08);
    let (item, _) = FeedbagItem::parse(&snac.body).unwrap();
    assert_eq!(item.name, "Buddy1");
    assert_eq!(item.class_id, FeedbagItem::CLASS_DENY);

    // Client removes Buddy1 from the block list (subtype 0x0A "delete item").
    let unblock_request = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&unblock_request.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Feedbag.as_u16());
    assert_eq!(snac.header.subtype, 0x0A);
    let (item, _) = FeedbagItem::parse(&snac.body).unwrap();
    assert_eq!(item.name, "Buddy1");
    assert_eq!(item.class_id, FeedbagItem::CLASS_DENY);
}

#[tokio::test]
async fn feedbag_presence_messaging_and_away_status_round_trip() {
    let bos_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bos_port = bos_listener.local_addr().unwrap().port();
    let bos_address_str = format!("127.0.0.1:{bos_port}");

    let auth_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let auth_port = auth_listener.local_addr().unwrap().port();

    let bos_task = tokio::spawn(async move {
        let (stream, _) = bos_listener.accept().await.unwrap();
        let conn = FlapConnection::from_stream(stream);
        serve_bos_session(conn).await;
    });

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

    let server = ServerAddress::parse(&format!("127.0.0.1:{auth_port}")).unwrap();
    let mut session = login(&server, "TestScreenName", "hunter2").await.expect("login should succeed against the fake server");

    // Feedbag reply + ack.
    session.handle_next_frame().await.unwrap();
    assert_eq!(
        session.buddies,
        vec![Buddy {
            screen_name: "Buddy1".to_string(),
            group_name: "Buddies".to_string(),
            is_online: false,
            is_away: false,
            away_message: None,
            warning_level: 0,
            is_blocked: false,
        }]
    );

    // Presence arrival, away, and already-warned.
    session.handle_next_frame().await.unwrap();
    assert!(session.buddies[0].is_online);
    assert!(session.buddies[0].is_away);
    assert_eq!(session.buddies[0].warning_level, 250);

    // Incoming IM.
    session.handle_next_frame().await.unwrap();
    assert_eq!(session.incoming_messages.len(), 1);
    assert_eq!(session.incoming_messages[0].from, "Buddy1");
    assert_eq!(session.incoming_messages[0].text, "hey there");

    // Reply, go away, and ask Buddy1 what they're up to.
    session.send_message("Buddy1", "hi back").await.unwrap();
    session.set_away_message(Some("brb")).await.unwrap();
    assert_eq!(session.away_message.as_deref(), Some("brb"));
    session.request_user_info("Buddy1").await.unwrap();

    // Buddy1's away-message reply.
    session.handle_next_frame().await.unwrap();
    assert_eq!(session.buddies[0].away_message.as_deref(), Some("On a call"));

    // Warn Buddy1, then block and unblock them.
    session.send_warning("Buddy1", false).await.unwrap();
    session.handle_next_frame().await.unwrap();
    assert_eq!(session.buddies[0].warning_level, 500);

    session.add_to_block_list("Buddy1").await.unwrap();
    assert!(session.buddies[0].is_blocked);
    session.remove_from_block_list("Buddy1").await.unwrap();
    assert!(!session.buddies[0].is_blocked);

    auth_task.await.unwrap();
    bos_task.await.unwrap();
}
