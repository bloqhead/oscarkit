//! Since this environment can't reach a real Open OSCAR Server instance,
//! this spins up a tiny fake one locally — just enough of the auth + BOS
//! handshake to exercise `login()` end to end. This proves the async state
//! machine actually completes (connects, round-trips four SNACs, handles
//! the auth->BOS handoff) without deadlocking or panicking. It does NOT
//! prove the byte layouts match a real server's expectations — that still
//! needs a Wireshark capture against Pidgin talking to the actual Hetzner
//! box, same caveat as everywhere else in this project.

use oscar_rs::{login, FlapConnection, ServerAddress, Snac, SnacFamily, SnacHeader, Tlv};
use tokio::net::TcpListener;

/// Runs one fake auth-server exchange on an already-accepted connection.
async fn serve_auth(mut conn: FlapConnection, bos_address: &str) {
    // Client hello (channel 1) — ignore the content, just consume the frame.
    conn.read_frame().await.unwrap();

    // Client requests an auth key (family 0x17, subtype 0x06).
    let request = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&request.payload).unwrap();
    assert_eq!(snac.header.family, SnacFamily::Authorization.as_u16());
    assert_eq!(snac.header.subtype, 0x06);

    // Reply with a fixed auth key. Per Open OSCAR Server's own source
    // (wire.SNAC_0x17_0x07_BUCPChallengeResponse), this body is NOT a TLV —
    // it's a plain `len_prefix=uint16` string: 2-byte length + raw bytes.
    let auth_key = b"fake-challenge-bytes".to_vec();
    let mut reply_body = (auth_key.len() as u16).to_be_bytes().to_vec();
    reply_body.extend_from_slice(&auth_key);
    let reply = Snac {
        header: SnacHeader { family: SnacFamily::Authorization.as_u16(), subtype: 0x07, flags: 0, request_id: 1 },
        body: reply_body,
    };
    conn.send_snac(&reply).await.unwrap();

    // Client sends the login request (family 0x17, subtype 0x02) with the roasted hash.
    let login_request = conn.read_frame().await.unwrap().unwrap();
    let snac = Snac::parse(&login_request.payload).unwrap();
    assert_eq!(snac.header.subtype, 0x02);
    let tlvs = Tlv::parse_all(&snac.body);
    assert!(tlvs.contains_key(&0x25), "login request must include the roasted password hash TLV");

    // Reply with success: BOS address + a cookie.
    let mut body = Vec::new();
    body.extend(Tlv::new(0x05, bos_address.as_bytes().to_vec()).encode());
    body.extend(Tlv::new(0x06, b"fake-session-cookie".to_vec()).encode());
    let reply = Snac {
        header: SnacHeader { family: SnacFamily::Authorization.as_u16(), subtype: 0x03, flags: 0, request_id: 2 },
        body,
    };
    conn.send_snac(&reply).await.unwrap();
}

/// Runs one fake BOS-server exchange: accept the cookie-bearing hello, then
/// announce "host online" so the client's login completes.
async fn serve_bos(mut conn: FlapConnection) {
    // Client hello carrying the auth cookie — ignore content, just consume it.
    conn.read_frame().await.unwrap();

    let reply = Snac {
        header: SnacHeader { family: SnacFamily::Generic.as_u16(), subtype: 0x03, flags: 0, request_id: 1 },
        body: Vec::new(),
    };
    conn.send_snac(&reply).await.unwrap();
}

#[tokio::test]
async fn full_login_flow_completes_against_fake_server() {
    let bos_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let bos_port = bos_listener.local_addr().unwrap().port();
    let bos_address_str = format!("127.0.0.1:{bos_port}");

    let auth_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let auth_port = auth_listener.local_addr().unwrap().port();

    // Fake BOS server: accept one connection, run the handshake.
    let bos_task = tokio::spawn(async move {
        let (stream, _) = bos_listener.accept().await.unwrap();
        let conn = FlapConnection::from_stream(stream);
        serve_bos(conn).await;
    });

    // Fake auth server: accept one connection, run the handshake.
    let bos_address_for_auth = bos_address_str.clone();
    let auth_task = tokio::spawn(async move {
        let (stream, _) = auth_listener.accept().await.unwrap();
        let conn = FlapConnection::from_stream(stream);
        serve_auth(conn, &bos_address_for_auth).await;
    });

    let server = ServerAddress::parse(&format!("127.0.0.1:{auth_port}")).unwrap();
    let session = login(&server, "TestScreenName", "hunter2").await.expect("login should succeed against the fake server");

    assert_eq!(session.screen_name, "TestScreenName");

    auth_task.await.unwrap();
    bos_task.await.unwrap();
}

#[tokio::test]
async fn login_surfaces_server_rejection_as_an_error() {
    let auth_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let auth_port = auth_listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let (stream, _) = auth_listener.accept().await.unwrap();
        let mut conn = FlapConnection::from_stream(stream);

        conn.read_frame().await.unwrap(); // hello
        conn.read_frame().await.unwrap(); // auth key request

        let auth_key_reply = Snac {
            header: SnacHeader { family: SnacFamily::Authorization.as_u16(), subtype: 0x07, flags: 0, request_id: 1 },
            body: {
                let mut body = 3u16.to_be_bytes().to_vec(); // "key".len()
                body.extend_from_slice(b"key");
                body
            },
        };
        conn.send_snac(&auth_key_reply).await.unwrap();

        conn.read_frame().await.unwrap(); // login request

        // Reply with an error instead of a BOS handoff — simulates a bad password.
        let error_reply = Snac {
            header: SnacHeader { family: SnacFamily::Authorization.as_u16(), subtype: 0x03, flags: 0, request_id: 2 },
            body: Tlv::new(0x08, 0x0004u16.to_be_bytes().to_vec()).encode(), // arbitrary BUCP error code
        };
        conn.send_snac(&error_reply).await.unwrap();
    });

    let server = ServerAddress::parse(&format!("127.0.0.1:{auth_port}")).unwrap();
    let result = login(&server, "TestScreenName", "wrongpassword").await;

    assert!(matches!(result, Err(oscar_rs::OscarError::LoginFailed(_))));
}
