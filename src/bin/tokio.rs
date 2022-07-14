use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

async fn process_socket(mut socket: TcpStream) {
    let mut req = vec![0; 4096];
    let res = b"HTTP/1.1 200 OK\r\nContent-length: 12\r\n\r\nHello world\n";

    loop {
        let n = socket.read(&mut req).await.unwrap();
        if n == 0 {
            return;
        }
        socket.write(res).await.unwrap();
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tokio::spawn(async {
        let listener = TcpListener::bind("127.0.0.1:9000").await.unwrap();

        loop {
            let (socket, _) = listener.accept().await.unwrap();
            socket.set_nodelay(true).unwrap();
            tokio::spawn(process_socket(socket));
        }
    }).await.unwrap();
}