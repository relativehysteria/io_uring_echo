use std::io;
use std::cmp::Ordering;
use io_uring_echo::EchoServer;

const PORT: u16        = 6969;
const CONNECTIONS: u16 = 2;

fn fork(env: impl Fn() -> io::Result<()>) -> io::Result<i32> {
    extern "C" {
        fn fork() -> i32;
    }

    let pid = unsafe { fork() };

    match pid.cmp(&0) {
        Ordering::Equal   => (),
        Ordering::Greater => return Ok(pid),
        Ordering::Less    => return Err(io::Error::last_os_error()),
    }

    env()?;
    std::process::exit(0);
}

fn main() -> io::Result<()> {
    // Validate our defaults and shit
    assert!(CONNECTIONS.count_ones() == 1, "CONNECTIONS must be a power of 2");

    println!("PARENT PROCESS: {}", std::process::id());

    let fork_res = fork(|| -> io::Result<()> {
        let mut server = EchoServer::new(CONNECTIONS, PORT)?;
        loop {
            server.tick()?;
            print!(","); // No flushing
        }
    });
    let fork_res2 = fork(|| -> io::Result<()> {
        let mut server = EchoServer::new(CONNECTIONS, PORT+1)?;
        loop {
            server.tick()?;
            print!("."); // No flushing
        }
    });

    println!("Got two forks: {fork_res:?} {fork_res2:?}");

    // Just park the thread. We don't care about forks crashing. Or anything.
    std::thread::park();

    Ok(())
}
