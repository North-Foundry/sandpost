use sandpost_mail::serve;
use std::time::Duration;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::oneshot,
    time::timeout,
};

/// Enforce the public connection ceiling and drain admitted conversations after shutdown.
#[tokio::test]
async fn server_limits_connections_and_drains_existing_sessions_on_shutdown() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listen_address = listener.local_addr().unwrap();
    let (shutdown_sender, shutdown_receiver) = oneshot::channel();
    let server_task = tokio::spawn(serve(
        listener,
        |_| async { Ok::<(), std::io::Error>(()) },
        async {
            let _ = shutdown_receiver.await;
        },
    ));

    let mut clients = Vec::new();
    for _ in 0..32 {
        let stream = TcpStream::connect(listen_address).await.unwrap();
        let mut client = BufReader::new(stream);
        let mut greeting = String::new();
        timeout(Duration::from_secs(2), client.read_line(&mut greeting))
            .await
            .unwrap()
            .unwrap();
        assert!(greeting.starts_with("220"));
        clients.push(client);
    }

    let mut rejected_client = BufReader::new(TcpStream::connect(listen_address).await.unwrap());
    let mut response = String::new();
    assert_eq!(
        timeout(
            Duration::from_secs(2),
            rejected_client.read_line(&mut response)
        )
        .await
        .unwrap()
        .unwrap(),
        0,
    );

    shutdown_sender.send(()).unwrap();
    for client in &mut clients {
        client.get_mut().write_all(b"QUIT\r\n").await.unwrap();
        response.clear();
        timeout(Duration::from_secs(2), client.read_line(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with("221"));
    }
    timeout(Duration::from_secs(2), server_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}
