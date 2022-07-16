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
    // pin ourself to cpu 0, good for cpu cache and maybe numa
    affinity::set_thread_affinity([0]).unwrap();

    tokio::spawn(async {
        let listener = TcpListener::bind("[::1]:9000").await.unwrap();

        loop {
            let (socket, _) = listener.accept().await.unwrap();

            let sref = socket2::SockRef::from(&socket);

            // set the processor rx queue for this socket to be associated with cpu 0, which we
            // are pinned to. good for numa and cpu cache
            sref.set_cpu_affinity(0).unwrap();

            socket.set_nodelay(true).unwrap();
            tokio::spawn(process_socket(socket));
        }
    })
        .await
        .unwrap();
}
