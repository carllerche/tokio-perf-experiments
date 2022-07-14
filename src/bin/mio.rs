use mio::net::{TcpListener, TcpStream};
use mio::Interest;
use mio::{Events, Poll, Token};
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::{Read, Write};

fn main() {
    let addr = "127.0.0.1:9000".parse().unwrap();
    let mut listener = TcpListener::bind(addr).unwrap();

    let mut poll = Poll::new().unwrap();
    poll.registry()
        .register(&mut listener, Token(0), Interest::READABLE)
        .unwrap();

    let mut counter: usize = 0;
    let mut sockets: HashMap<Token, TcpStream> = HashMap::new();
    let mut buffer = [0 as u8; 1024];

    let response = b"HTTP/1.1 200 OK\r\nContent-length: 12\r\n\r\nHello world\n";

    let mut events = Events::with_capacity(1024);
    loop {
        poll.poll(&mut events, None).unwrap();
        for event in &events {
            match event.token() {
                Token(0) => loop {
                    match listener.accept() {
                        Ok((mut socket, _)) => {
                            counter += 1;
                            let token = Token(counter);

                            poll.registry()
                                .register(&mut socket, token, Interest::READABLE)
                                .unwrap();
                            sockets.insert(token, socket);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                },
                token if event.is_readable() => {
                    let socket = sockets.get_mut(&token).unwrap();
                    let read = socket.read(&mut buffer);
                    match read {
                        Ok(0) => {
                            sockets.remove(&token);
                            continue;
                        }
                        Ok(_n) => {}
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            println!("socket  read WouldBlock");
                            break;
                        }
                        Err(_) => break,
                    }

                    let n = socket.write(response).unwrap();
                    assert_eq!(n, response.len());
                }
                _token if event.is_writable() => {
                    unreachable!();
                }
                _ => unreachable!(),
            }
        }
    }
}
