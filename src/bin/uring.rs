use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::net::SocketAddr;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::{io, mem, ptr};

use io_uring::types::Fd;
use io_uring::{cqueue, opcode, squeue, CompletionQueue, SubmissionQueue, Submitter};
use libc::{c_int, sockaddr, socklen_t};
use slab::Slab;
use socket2::Socket;

const RESPONSE: &'static [u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nHello world!\n";

const BACKLOG: c_int = 256;
const URING_QUEUE_SIZE: u32 = 2048;

// For building the buffer pool
const BUF_CNT: usize = 128;
const BUF_LEN: usize = 4096;
const GROUP_ID: u16 = 1337;

#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq)]
#[repr(u32)]
enum EventKind {
    Accept = 0,
    Recv = 1,
    Send = 2,
    ProvideBuf = 4,
}

#[repr(packed)]
#[derive(Clone, Copy)]
struct UserData {
    kind: EventKind,
    id: u32,
}

struct AddrPinned {
    addr: UnsafeCell<MaybeUninit<sockaddr>>,
    len: UnsafeCell<MaybeUninit<socklen_t>>,
}

fn main() {
    // Create the buffer pool
    let mut bufs = vec![0; BUF_CNT * BUF_LEN];
    assert_eq!(bufs.len(), BUF_CNT * BUF_LEN);

    let listener = socket2::Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )
    .unwrap();
    listener.set_reuse_address(true).unwrap();

    let addr: SocketAddr = "[::1]:9000".parse().unwrap();

    listener.bind(&addr.into()).unwrap();

    listener.listen(BACKLOG).unwrap();

    let mut uring = io_uring::IoUring::new(URING_QUEUE_SIZE).unwrap();

    uring.submission();

    let (mut submitter, mut squeue, mut completions) = uring.split();

    // Prepare buffer pool
    unsafe { add_all_bufs(&mut submitter, &mut squeue, &mut bufs).unwrap() };
    submitter.submit_and_wait(1).unwrap();
    completions.sync();

    let mut sockets = Slab::with_capacity(1024);

    for c in &mut completions {
        assert_eq!(c.result(), 0);
    }

    for _ in 0..64 {
        let addr = Box::new(AddrPinned {
            addr: UnsafeCell::new(MaybeUninit::uninit()),
            len: UnsafeCell::new(MaybeUninit::uninit()),
        });

        unsafe { submit_accept(&listener, &mut submitter, &mut squeue, addr).unwrap() }
    }

    loop {
        unsafe {
            handle_completions(
                &listener,
                &mut submitter,
                &mut squeue,
                &mut completions,
                &mut sockets,
                &bufs,
            );
        }
    }
}

unsafe fn add_all_bufs(
    submitter: &mut Submitter,
    squeue: &mut SubmissionQueue,
    bufs: &mut Vec<u8>,
) -> io::Result<()> {
    let entry =
        opcode::ProvideBuffers::new(bufs.as_mut_ptr(), BUF_LEN as _, BUF_CNT as _, GROUP_ID, 0)
            .build();

    while squeue.push(&entry).is_err() {
        // submitter.submit().unwrap();
        panic!();
    }

    squeue.sync();
    Ok(())
}

unsafe fn handle_completions(
    listener: &socket2::Socket,
    submitter: &mut Submitter,
    squeue: &mut SubmissionQueue,
    completions: &mut CompletionQueue,
    sockets: &mut Slab<Socket>,
    buf: &[u8],
) {
    submitter.submit_and_wait(1).unwrap();
    completions.sync();

    for c in completions {
        assert!(sockets.len() < 100);
        let user_data: UserData = mem::transmute(c.user_data());

        match user_data.kind {
            EventKind::Accept => {
                if c.result() > 0 {
                    let sock = Socket::from_raw_fd(c.result());

                    let addr = Box::from_raw(user_data.id as _);

                    let entry = sockets.vacant_entry();
                    submit_recv(&sock, entry.key() as _, submitter, squeue).unwrap();
                    submit_accept(&listener, submitter, squeue, addr).unwrap();
                    entry.insert(sock);
                }
            }
            EventKind::Recv => {
                if let Some(bid) = cqueue::buffer_select(c.flags()) {
                    let pos = bid as usize * BUF_LEN;
                    let buffer = &buf[pos..(pos + BUF_LEN)];

                    // Add the buffer back
                    let entry = opcode::ProvideBuffers::new(
                        buffer.as_ptr() as _,
                        BUF_LEN as _,
                        1,
                        GROUP_ID,
                        bid,
                    )
                    .build()
                    .user_data(mem::transmute(UserData {
                        kind: EventKind::ProvideBuf,
                        id: 0,
                    }));

                    while squeue.push(&entry).is_err() {
                        // submitter.submit().unwrap();
                        panic!();
                    }

                    if c.result() <= 0 {
                        // submit_close(&sockets[user_data.id as _], user_data.id, submitter, squeue).unwrap();
                        let sock = sockets.remove(user_data.id as _);
                        libc::close(sock.as_raw_fd());
                    } else {
                        submit_send(&sockets[user_data.id as _], user_data.id, submitter, squeue)
                            .unwrap();

                        // And submit another recv
                        submit_recv(&sockets[user_data.id as _], user_data.id, submitter, squeue).unwrap();
                    }
                }
            }
            EventKind::Send => {
                // submit_recv(&sockets[user_data.id as _], user_data.id, submitter, squeue).unwrap();
            }
            EventKind::ProvideBuf => {}
        }
    }

    squeue.sync();
}

unsafe fn submit_accept(
    listener: &socket2::Socket,
    submitter: &mut Submitter,
    squeue: &mut SubmissionQueue,
    // data: UserData,
    addr: Box<AddrPinned>,
) -> io::Result<()> {
    let entry = opcode::Accept::new(
        Fd(listener.as_raw_fd()),
        addr.addr.get() as _,
        addr.len.get() as _,
    )
    .build()
    .user_data(mem::transmute(UserData {
        kind: EventKind::Accept,
        id: addr.addr.get() as _,
    }));

    mem::forget(addr);

    while squeue.push(&entry).is_err() {
        // submitter.submit().unwrap();
        panic!();
    }

    squeue.sync();
    Ok(())
}

unsafe fn submit_recv(
    stream: &Socket,
    token: u32,
    sub: &mut Submitter,
    squeue: &mut SubmissionQueue,
) -> io::Result<()> {
    let entry = opcode::Recv::new(Fd(stream.as_raw_fd()), ptr::null_mut(), BUF_LEN as _)
        .buf_group(GROUP_ID)
        .build()
        .flags(squeue::Flags::BUFFER_SELECT)
        .user_data(mem::transmute(UserData {
            kind: EventKind::Recv,
            id: token,
        }));

    while squeue.push(&entry).is_err() {
        sub.submit().unwrap();
        panic!();
    }

    Ok(())
}

unsafe fn submit_send(
    stream: &socket2::Socket,
    id: u32,
    submitter: &mut Submitter,
    squeue: &mut SubmissionQueue,
) -> io::Result<()> {
    let user_data = UserData {
        kind: EventKind::Send,
        id,
    };

    let entry = opcode::Send::new(
        Fd(stream.as_raw_fd()),
        RESPONSE.as_ptr(),
        RESPONSE.len() as u32,
    )
    .build()
    .user_data(mem::transmute(user_data));

    while squeue.push(&entry).is_err() {
        panic!();
        // submitter.submit().unwrap();
    }

    Ok(())
}
