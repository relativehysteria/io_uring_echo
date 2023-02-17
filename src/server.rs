use std::net::TcpListener;
use std::os::fd::{RawFd, AsRawFd};
use std::io;
use std::ptr::null_mut;
use io_uring::types::Fd;
use io_uring::{IoUring, opcode};
use crate::Slab;

#[allow(dead_code)]
#[derive(Clone, Debug)]
enum OpType {
    Accept,
    Poll  { fd: RawFd, },
    Read  { fd: RawFd, buf: Box<[u8]> },
    Write { fd: RawFd, buf: Box<[u8]>, offset: usize, len: usize },
}

extern "C" {
    fn close(fd: i32) -> i32;
}

/// A server that echoes back everything it is sent.
pub struct EchoServer {
    /// The internal TcpListener
    _listener: TcpListener,

    /// The file descriptor of the internal TcpListener
    fd: Fd,

    /// The amount of `accept`s we have to put into the `SubmissionQueue`
    /// in the internal io_uring. This is a tracking variable that gets
    /// incremented on each `accept` push and decremented on completed `accept`s
    count: u16,

    /// The internal io_uring state. This isn't directly used and is only here
    /// so that references to submitter, sq and cq don't get dropped
    ring: IoUring,

    /// The mapping of tokens and their specific operations.
    token_ops: Slab<OpType>,
}

impl EchoServer {
    /// `count` - maximum number of connected clients.
    /// `port`  - port on which to start listening.
    ///
    /// The larger the `count`, the larger the internal io_uring queues.
    /// `count` must be a power of two.
    pub fn new(count: u16, port: u16) -> io::Result<Self> {
        // Validate the count
        assert!(count.is_power_of_two(), "`count` must be a power of 2.");

        // Create the rings
        let ring_size = u32::from(count) * 2;
        let ring = IoUring::new(ring_size)?;

        // Create the listener
        let _listener = TcpListener::bind(("0.0.0.0", port))?;
        let fd = Fd(_listener.as_raw_fd());

        // In the beginning, all tokens are `accept`s
        let mut token_ops = Slab::with_capacity(ring_size.try_into().unwrap());

        // The first spot in the slab is reserved for `accept` opcodes
        token_ops.insert(OpType::Accept);

        Ok(Self {
            _listener,
            fd,
            count,
            ring,
            token_ops,
        })
    }

    /// Returns the current amount of possible connections that are not pushed
    /// into the submission queue. This number effectively shows you how many
    /// connections *we are not accepting* but are supposed to.
    pub fn count(&self) -> u16 {
        self.count
    }

    /// Returns the primary TcpListener file desciptor of this server
    pub fn fd(&self) -> Fd {
        self.fd
    }

    /// Push as many `accept` opcodes into the submission ring as needed
    /// (based on `self.count()`).
    fn push_accepts(&mut self) {
        // Accept opcode
        let accept = opcode::Accept::new(self.fd, null_mut(), null_mut())
            .build()
            .user_data(0);

        // Get the submisison queue
        let mut sq = self.ring.submission();

        // Push as many accept opcodes into the queue as we can
        while self.count > 0 {
            unsafe {
                match sq.push(&accept) {
                    Ok(_)  => self.count -= 1,
                    Err(_) => break,
                }
            }
        }
        sq.sync();
    }

    /// Poll and handle the internal io_uring queues once. This is the function
    /// used in the poll loop of the server.
    pub fn tick(&mut self) -> io::Result<()> {
        const EBUSY: i32 = 16;
        const ECONNRESET: i32 = 104;
        const POLLIN: u32 = 1;

        // Make sure we can accepts connections
        self.push_accepts();

        // Split the ring into its internal components
        let (submitter, mut sq, mut cq) = self.ring.split();

        // Wait for the completion queue to have some entries
        match submitter.submit_and_wait(1) {
            Ok(_)    => (),
            Err(err) => match err.raw_os_error() {
                Some(EBUSY) => (),
                _ => Err(io::ErrorKind::Other)?,
            },
        }
        cq.sync();

        // TODO: Clean the backlog

        // Go through each completion queue entry
        for cqe in &mut cq {
            let ret = cqe.result();
            let usr = cqe.user_data().try_into().unwrap();

            // Log any errors
            if ret < 0 {
                let err = io::Error::from_raw_os_error(-ret);

                // Don't warn on errors like connection reset...
                match -ret {
                    ECONNRESET => (),
                    __________ => eprintln!("Token `{usr}` got error `{err}`"),
                }

                // Close the file descriptor if we have one
                match self.token_ops.get(usr).unwrap() {
                    OpType::Poll {fd} | OpType::Read {fd,..}
                    | OpType::Write {fd,..} => {
                        unsafe { close(*fd); }
                    },
                    _ => (),
                }

                // Mark the user_data as free to use
                self.token_ops.mark_free(usr);
                continue;
            }

            // Get the OpType of this event
            let optype = match self.token_ops.get(usr) {
                Some(optype) => optype,
                None => {
                    eprintln!("user_data {usr} not registered.");
                    continue;
                },
            };

            // Handle the operation.
            // XXX: Not too many comments from now on
            match optype.clone() {
                OpType::Accept => {
                    let token = self.token_ops.insert(OpType::Poll { fd: ret });

                    let poll = opcode::PollAdd::new(Fd(ret), POLLIN)
                        .build()
                        .user_data(token.try_into().unwrap());

                    // TODO: push to backlog
                    unsafe { sq.push(&poll).unwrap(); }

                    self.count += 1;
                },
                OpType::Poll { fd } => {
                    let mut buf = vec![0u8; 4096].into_boxed_slice();

                    let read = opcode::Recv::new(Fd(fd), buf.as_mut_ptr(),
                                                 buf.len().try_into().unwrap())
                        .build()
                        .user_data(usr.try_into().unwrap());

                    let token = match self.token_ops.get_mut(usr) {
                        Some(token) => token,
                        None        => continue,
                    };

                    *token = OpType::Read { fd, buf };

                    // TODO: push to backlog
                    unsafe { sq.push(&read).unwrap(); }
                },
                OpType::Read { fd, buf } => {
                    if ret == 0 {
                        println!("exit");
                        self.token_ops.mark_free(usr);
                        unsafe { close(fd); }
                        continue;
                    }

                    let write = opcode::Send::new(Fd(fd), buf.as_ptr(),
                                                  buf.len().try_into().unwrap())
                        .build()
                        .user_data(usr.try_into().unwrap());

                    let token = match self.token_ops.get_mut(usr) {
                        Some(token) => token,
                        None        => continue,
                    };

                    let len = ret.try_into().unwrap();
                    *token = OpType::Write { fd, buf, offset: 0, len };

                    // TODO: push to backlog
                    unsafe { sq.push(&write).unwrap(); }
                },
                OpType::Write { fd, buf, offset, len } => {
                    let write_len: usize = ret.try_into().unwrap();

                    let token = match self.token_ops.get_mut(usr) {
                        Some(token) => token,
                        None        => continue,
                    };

                    if offset + write_len >= len {
                        let poll = opcode::PollAdd::new(Fd(fd), POLLIN)
                            .build()
                            .user_data(usr.try_into().unwrap());
                        *token = OpType::Poll { fd };
                        unsafe { sq.push(&poll).unwrap(); }
                        continue;
                    }

                    let write = opcode::Send::new(Fd(fd), buf.as_ptr(),
                            buf.len().try_into().unwrap())
                        .build()
                        .user_data(usr.try_into().unwrap());
                    *token = OpType::Write { fd, buf, offset, len };

                    // TODO: push to backlog
                    unsafe { sq.push(&write).unwrap(); }
                },
            }
        }

        Ok(())
    }
}
