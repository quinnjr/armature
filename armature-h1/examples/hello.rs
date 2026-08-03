//! A minimal `armature-h1` server.
//!
//! ```sh
//! cargo run -p armature-h1 --release --example hello
//! curl -v http://127.0.0.1:8080/
//! ```

use armature_h1::{Config, Request, Response, Server};
use std::cell::Cell;
use std::net::SocketAddr;
use std::rc::Rc;

fn main() -> std::io::Result<()> {
    let addr: SocketAddr = "127.0.0.1:8080".parse().expect("valid address");
    let server = Server::bind(Config::new(addr).server_name("armature-h1".into()))?;
    println!("listening on http://{}", server.local_addr());

    // One counter per worker thread. Under thread-per-core nothing migrates
    // cores, so this is a plain `Cell` — no atomic, no lock. That is the whole
    // point of the model, and it is why handler futures need not be `Send`.
    server.serve(|| {
        let served = Rc::new(Cell::new(0u64));
        move |_req: Request| {
            let served = served.clone();
            async move {
                served.set(served.get() + 1);
                Response::text("Hello, world!")
            }
        }
    })
}
