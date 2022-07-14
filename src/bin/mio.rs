use mio::net::{TcpListener, TcpStream};
use mio::Interest;
use mio::{Events, Poll, Token};
use std::io::{Read, Write};

use std::os::unix::io::AsRawFd;

const RESP: &[u8] = b"HTTP/1.1 200 OK\r\nContent-length: 12\r\n\r\nHello world\n";

fn main() {
    let addr = "127.0.0.1:9000".parse().unwrap();
    let mut listener = TcpListener::bind(addr).unwrap();

    let mut poll = Poll::new().unwrap();
    poll.registry()
        .register(&mut listener, Token(1024), Interest::READABLE)
        .unwrap();

    let mut sockets = slab::Slab::with_capacity(1024);
    let mut buffer = vec![0 as u8; 1024 * 128];

    let mut events = Events::with_capacity(1024);
    loop {
        poll.poll(&mut events, None).unwrap();
        for event in &events {
            match event.token() {
                Token(1024) => loop {
                    match listener.accept() {
                        Ok((mut socket, _)) => {

                            socket.set_nodelay(true).unwrap();

                            let entry = sockets.vacant_entry();
                            let token = Token(entry.key());

                            poll.registry()
                                .register(&mut socket, token, Interest::READABLE)
                                .unwrap();

                            entry.insert(socket);
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(_) => break,
                    }
                },
                token if event.is_readable() => {
                    if event.is_read_closed() {
                        sockets.remove(token.0);
                    } else if process(&mut sockets[token.0], &mut buffer[..]) {
                        sockets.remove(token.0);
                    }
                }
                _ => unreachable!(),
            }
        }
    }
}

fn process(socket: &mut TcpStream, buffer: &mut [u8]) -> bool {
    let read = socket.read(&mut buffer[..]);
    match read {
        Ok(0) => {
            return true;
        }
        Ok(_) => {}
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            panic!();
        }
        Err(_) => {
            return true;
        }
    }

    let n = unsafe { libc::send(socket.as_raw_fd(), RESP.as_ptr() as _, RESP.len(), libc::MSG_NOSIGNAL) } as usize;

    assert_eq!(n, RESP.len());
    false
}
