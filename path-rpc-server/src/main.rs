extern crate antidote;
extern crate bytes;
extern crate failure;
extern crate futures;
extern crate http;
extern crate httparse;
#[macro_use]
extern crate lazy_static;
extern crate num_cpus;
extern crate path_rpc_common as rpc;
extern crate petgraph;
#[macro_use]
extern crate serde_json;
extern crate time;
extern crate tokio_core;
extern crate tokio_io;

use std::env;
use std::fmt;
use std::io;
use std::net::{self, SocketAddr};
use std::str;
use std::sync::Arc;
use std::thread;

use antidote::Mutex;
use bytes::{Bytes, BytesMut};
use failure::{Error, Fail};
use futures::future;
use futures::sync::mpsc;
use futures::{Future, Sink, Stream};
use http::header::HeaderValue;
use http::{Method, Request, Response, StatusCode};
use petgraph::algo::astar;
use petgraph::graphmap::DiGraphMap;
use petgraph::visit::EdgeRef;
use serde_json::Value;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;
use tokio_io::codec::{Decoder, Encoder};
use tokio_io::AsyncRead;

lazy_static! {
    /// Maintain the global state of the paths so that we can reference it wherever it is needed.
    pub static ref STATE: State = { State::new() };
}

fn main() -> Result<(), Error> {
    // Parse the arguments, bind the TCP socket we'll be listening to, spin up
    // our worker threads, and start shipping sockets to those worker threads.
    let addr = env::args()
        .nth(1)
        .unwrap_or("0.0.0.0:8080".to_string())
        .parse::<SocketAddr>()?;
    let num_threads = env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(num_cpus::get());

    let listener = net::TcpListener::bind(&addr)?;
    println!("Listening on: {}", addr);

    let mut channels = Vec::new();
    for _ in 0..num_threads {
        let (tx, rx) = mpsc::unbounded();
        channels.push(tx);
        thread::spawn(|| worker(rx));
    }

    let mut next = 0;
    for socket in listener.incoming() {
        if let Ok(socket) = socket {
            channels[next].unbounded_send(socket)?;
            next = (next + 1) % channels.len();
        }
    }

    Ok(())
}

/// How much spawning a car will increase crowdedness on a path
const CAR: u64 = 1;

#[derive(Clone)]
pub struct State {
    paths: Arc<Mutex<DiGraphMap<&'static str, u64>>>,
}

impl State {
    fn new() -> Self {
        // Unfortunately this is manual for now. Come up with something better
        let mut map = DiGraphMap::new();
        map.add_edge("New York City", "Boston", 3);
        map.add_edge("New York City", "Providence", 2);
        map.add_edge("Providence", "Boston", 1);
        map.add_edge("Providence", "New York City", 3);
        map.add_edge("Boston", "New York City", 5);
        map.add_edge("Boston", "Bar Harbor", 6);
        map.add_edge("Bar Harbor", "Boston", 5);
        Self {
            paths: Arc::new(Mutex::new(map)),
        }
    }
}

fn worker(rx: mpsc::UnboundedReceiver<net::TcpStream>) -> Result<(), Error> {
    let mut core = Core::new()?;
    let handle = core.handle();

    let done = rx.for_each(move |socket| {
        // Associate each socket we get with our local event loop, and then use
        // the codec support in the tokio-io crate to deal with discrete
        // request/response types instead of bytes. Here we'll just use our
        // framing defined below and then use the `send_all` helper to send the
        // responses back on the socket after we've processed them
        let socket = future::result(TcpStream::from_stream(socket, &handle));
        let req = socket.and_then(|socket| {
            let (tx, rx) = socket.framed(Http).split();
            tx.send_all(rx.and_then(respond))
        });
        handle.spawn(req.then(move |result| {
            drop(result);
            Ok(())
        }));
        Ok(())
    });

    core.run(done).map_err(|_| {
        io::Error::new(
            io::ErrorKind::Other,
            "Worker failed to execute future properly",
        )
    })?;

    Ok(())
}

/// "Server logic" is implemented in this function.
fn respond(req: Request<Bytes>) -> impl Future<Item = Response<String>, Error = io::Error> {
    let mut ret = Response::builder();
    let body = {
        let mut bod = String::new();

        if *req.method() == Method::POST {
            match req.uri().path() {
                "/path" => {
                    ret.header("Content-Type", "application/json");

                    // Clone the state so we can have access to the paths
                    let state = STATE.clone();
                    let paths = state.paths.lock();

                    let body = str::from_utf8(req.body()).unwrap();
                    rpc::Request::from_json(body).unwrap();
                    if let Ok(req_json) = rpc::Request::from_json(body) {
                        let method = &req_json.method == "get_path";
                        let params = req_json.params.is_object();

                        if method && params {
                            if let (Some(Value::String(start)), Some(Value::String(end))) =
                                (req_json.params.get("start"), req_json.params.get("end"))
                            {
                                if let Some((time, path)) = astar(
                                    &*paths,
                                    &start,
                                    |finish| finish == end,
                                    |e| *e.weight(),
                                    |_| 0,
                                ) {
                                    match rpc::Response::new(
                                        json!({ "distance": time, "path": path }),
                                        0,
                                    ).to_json()
                                    {
                                        Ok(json) => bod = json,
                                        Err(e) => {
                                            return future::err(io::Error::new(
                                                io::ErrorKind::Other,
                                                e.compat(),
                                            ))
                                        }
                                    }
                                } else {
                                    match rpc::Response::new(
                                        json!("Invalid start or end city given"),
                                        0,
                                    ).to_json()
                                    {
                                        Ok(json) => bod = json,
                                        Err(e) => {
                                            return future::err(io::Error::new(
                                                io::ErrorKind::Other,
                                                e.compat(),
                                            ))
                                        }
                                    }
                                }
                            } else {
                                println!("Bad JSON input 1");
                                ret.status(StatusCode::BAD_REQUEST);
                            }
                        } else {
                            println!("Bad JSON input 2");
                            ret.status(StatusCode::BAD_REQUEST);
                        }
                    } else {
                        println!("Bad JSON input 3");
                        ret.status(StatusCode::BAD_REQUEST);
                    }
                }
                _ => {
                    println!("Not Found 1");
                    ret.status(StatusCode::NOT_FOUND);
                }
            }
        } else {
            println!("Not Found 2");
            ret.status(StatusCode::NOT_FOUND);
        }

        bod
    };
    future::ok(ret.body(body).unwrap())
}

struct Http;

/// Implementation of encoding an HTTP response into a `BytesMut`, basically
/// just writing out an HTTP/1.1 response.
impl Encoder for Http {
    type Item = Response<String>;
    type Error = io::Error;

    fn encode(&mut self, item: Response<String>, dst: &mut BytesMut) -> io::Result<()> {
        use std::fmt::Write;

        write!(
            BytesWrite(dst),
            "\
             HTTP/1.1 {}\r\n\
             Server: Example\r\n\
             Content-Length: {}\r\n\
             ",
            item.status(),
            item.body().len()
        ).unwrap();

        for (k, v) in item.headers() {
            dst.extend_from_slice(k.as_str().as_bytes());
            dst.extend_from_slice(b": ");
            dst.extend_from_slice(v.as_bytes());
            dst.extend_from_slice(b"\r\n");
        }

        dst.extend_from_slice(b"\r\n");
        dst.extend_from_slice(item.body().as_bytes());

        return Ok(());

        struct BytesWrite<'a>(&'a mut BytesMut);

        impl<'a> fmt::Write for BytesWrite<'a> {
            fn write_str(&mut self, s: &str) -> fmt::Result {
                self.0.extend_from_slice(s.as_bytes());
                Ok(())
            }

            fn write_fmt(&mut self, args: fmt::Arguments) -> fmt::Result {
                fmt::write(self, args)
            }
        }
    }
}

/// Implementation of decoding an HTTP request from the bytes we've read so far.
/// This leverages the `httparse` crate to do the actual parsing and then we use
/// that information to construct an instance of a `http::Request` object,
/// trying to avoid allocations where possible.
impl Decoder for Http {
    type Item = Request<Bytes>;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Request<Bytes>>> {
        // TODO: we should grow this headers array if parsing fails and asks
        //       for more headers
        let mut headers = [None; 16];
        let (method, path, version, amt, body_len) = {
            let mut parsed_headers = [httparse::EMPTY_HEADER; 16];
            let mut r = httparse::Request::new(&mut parsed_headers);
            let status = r.parse(src).map_err(|e| {
                let msg = format!("failed to parse http request: {:?}", e);
                io::Error::new(io::ErrorKind::Other, msg)
            })?;

            let amt = match status {
                httparse::Status::Complete(amt) => amt,
                httparse::Status::Partial => return Ok(None),
            };

            let toslice = |a: &[u8]| {
                let start = a.as_ptr() as usize - src.as_ptr() as usize;
                assert!(start < src.len());
                (start, start + a.len())
            };

            let mut body = 0;

            for (i, header) in r.headers.iter().enumerate() {
                if header.name == "Content-Length" {
                    // We know Content-Length will always be a number and valid utf8 so this is
                    // okay. This assumes that no one sends a malformed payload however.
                    body = unsafe { str::from_utf8_unchecked(header.value) }
                        .parse::<usize>()
                        .unwrap();
                }
                let k = toslice(header.name.as_bytes());
                let v = toslice(header.value);
                headers[i] = Some((k, v));
            }

            (
                toslice(r.method.unwrap().as_bytes()),
                toslice(r.path.unwrap().as_bytes()),
                r.version.unwrap(),
                amt,
                body,
            )
        };
        if version != 1 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "Only HTTP/1.1 accepted",
            ));
        }
        let data = src.split_to(amt).freeze();
        let body = src.split_to(body_len).freeze();
        let mut ret = Request::builder();
        ret.method(&data[method.0..method.1]);
        ret.uri(data.slice(path.0, path.1));
        ret.version(http::Version::HTTP_11);
        for header in headers.iter() {
            let (k, v) = match *header {
                Some((ref k, ref v)) => (k, v),
                None => break,
            };
            let value = unsafe { HeaderValue::from_shared_unchecked(data.slice(v.0, v.1)) };
            ret.header(&data[k.0..k.1], value);
        }

        let req = ret.body(body)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(Some(req))
    }
}
