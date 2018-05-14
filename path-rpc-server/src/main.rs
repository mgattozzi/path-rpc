extern crate antidote;
extern crate bytes;
extern crate chrono;
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
extern crate tokio_core;
extern crate tokio_io;

use std::{
    env, fmt, io, net::{self, SocketAddr}, str, sync::Arc, thread, time, u64,
};

use antidote::Mutex;
use bytes::{Bytes, BytesMut};
use chrono::{DateTime, Duration, Local};
use failure::{Error, Fail};
use futures::{future, sync::mpsc, Future, Sink, Stream};
use http::{header::HeaderValue, Method, Request, Response, StatusCode};
use petgraph::{algo::astar, graphmap::DiGraphMap, visit::EdgeRef};
use serde_json::Value;
use tokio_core::{net::TcpStream, reactor::Core};
use tokio_io::{
    codec::{Decoder, Encoder}, AsyncRead,
};

lazy_static! {
    /// Maintain the global state of the paths and journeys so that we can reference it wherever it is needed.
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

    // Spawn our thread that monitors the car traffic
    thread::spawn(|| {
        // Create a 5 second delay so that we're not always looping through what cars need to get
        // removed next and deadlocking everything
        let five_sec = time::Duration::from_millis(5000);
        let state = STATE.clone();

        // Keep looping till the server is killed
        loop {
            {
                let mut journeys = state.journey.lock();
                // Not great that we're reallocating every time here and could be improved in the
                // future. This was to avoid borrowing issues, but there's likely a better inplace
                // way to do this.
                *journeys = journeys
                    .iter()
                    .cloned()
                    .map(|(time, mut path)| {
                        if time >= Local::now() {
                            // Iterate over each segment to change the weighting
                            for nodes in path.as_slice().windows(2) {
                                let mut graph = state.paths.lock();
                                // We know these are valid paths else A* would not have worked
                                let edge = graph.edge_weight_mut(nodes[0], nodes[1]).unwrap();
                                // Subtract the crowdedness
                                *edge = *edge - CAR;
                            }
                            // Empty the vec to signal these are the ones we want to clear next
                            // time. Using time instead might cause the edge weight to increase and
                            // never clear out.
                            path.clear();
                        }
                        (time, path)
                    })
                    .filter(|(_, path)| path.len() > 0)
                    .collect();
            }
            // Try to avoid locking up resources too often by putting the thread to sleep
            thread::sleep(five_sec);
        }
    });

    // Spawn all of our workers
    for _ in 0..num_threads - 1 {
        let (tx, rx) = mpsc::unbounded();
        channels.push(tx);
        thread::spawn(|| worker(rx));
    }

    // As we get incoming connections send them to our workers
    let mut next = 0;
    for socket in listener.incoming() {
        if let Ok(socket) = socket {
            channels[next].unbounded_send(socket)?;
            next = (next + 1) % channels.len();
        }
    }

    Ok(())
}

/// How much spawning a car will increase crowdedness on a path per edge it travels over
const CAR: u64 = 1;

#[derive(Clone)]
pub struct State {
    /// A Directed Graph containing all possible Cities and their edges and weights
    paths: Arc<Mutex<DiGraphMap<u64, u64>>>,
    /// A buffer holding currently running paths to allow for crowdedness
    /// As cars are added they get put here, then when the journey is complete their paths
    /// have the time decreased
    journey: Arc<Mutex<Vec<(DateTime<Local>, Vec<u64>)>>>,
}

#[derive(Clone, Copy)]
/// An enum of all cities that exist for this server, where DNE is Does Not Exist which acts as a
/// catch all for bad requests
pub enum City {
    NewYorkCity,
    Boston,
    Providence,
    BarHarbor,
    DNE,
}

// Map a City to what it's NodeId reference is
impl Into<u64> for City {
    fn into(self) -> u64 {
        match self {
            City::NewYorkCity => 0,
            City::Boston => 1,
            City::Providence => 2,
            City::BarHarbor => 3,
            City::DNE => u64::MAX,
        }
    }
}

// Turn the strings sent in by request into the City type
impl<'a> From<&'a str> for City {
    fn from(item: &'a str) -> City {
        match item {
            "New York City" => City::NewYorkCity,
            "Boston" => City::Boston,
            "Providence" => City::Providence,
            "Bar Harbor" => City::BarHarbor,
            _ => City::DNE,
        }
    }
}

impl State {
    /// Initialize the server state
    fn new() -> Self {
        // Unfortunately this is manual for now. Future work would make this work based off reading
        // from a file or something like that
        let mut map = DiGraphMap::new();
        let nyc = City::NewYorkCity.into();
        let bos = City::Boston.into();
        let prov = City::Providence.into();
        let bh = City::BarHarbor.into();
        map.add_edge(nyc, bos, 3);
        map.add_edge(nyc, prov, 2);
        map.add_edge(prov, bos, 1);
        map.add_edge(prov, nyc, 3);
        map.add_edge(bos, nyc, 5);
        map.add_edge(bos, bh, 6);
        map.add_edge(bos, prov, 1);
        map.add_edge(bh, bos, 5);
        Self {
            paths: Arc::new(Mutex::new(map)),
            journey: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

/// Create a worker to handle incoming requests
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

/// "Server logic" is implemented in this function. Given an HTTP Request it then figures out
/// whether the request is valid, and if it is calculates the shortest path possible and returns
/// that value to the user
fn respond(req: Request<Bytes>) -> impl Future<Item = Response<String>, Error = io::Error> {
    let mut ret = Response::builder();
    let body = {
        // We always return either an empty body or some JSON and this is the buffer it gets put in
        let mut bod = String::new();

        // If the request is a POST request to the /path endpoint check to see if it's valid
        if *req.method() == Method::POST {
            match req.uri().path() {
                "/path" => {
                    ret.header("Content-Type", "application/json");

                    // Clone the state so we can have access to the paths
                    let state = STATE.clone();
                    let mut paths = state.paths.lock();

                    // This is pretty lose and it would be best to return an RPC response here on
                    // failure, but we can probably consider this okay for the purposes of an
                    // example as Rust sends utf8 strings only
                    let body = str::from_utf8(req.body()).unwrap();

                    // Parse the request and check that it is valid
                    if let Ok(req_json) = rpc::Request::from_json(body) {
                        // json-rpc allows us to set a method. We can use this as a way to send
                        // many requests to one endpoint but have it act differently based off the
                        // type of method asked for
                        let method = &req_json.method == "get_path";
                        // Due to jsons loosely typed nature we need to make sure we get an object
                        // here
                        let params = req_json.params.is_object();

                        if method && params {
                            if let (Some(Value::String(start)), Some(Value::String(end))) =
                                (req_json.params.get("start"), req_json.params.get("end"))
                            {
                                // Get the start and end NodeIds from the input
                                let start: u64 = City::from(start.as_str()).into();
                                let end: u64 = City::from(end.as_str()).into();

                                // Find the shortest path if possible
                                if let Some((time, path)) = astar(
                                    &*paths,
                                    start,
                                    |finish: u64| finish == end,
                                    |e| *e.weight(),
                                    |_| 0,
                                ) {
                                    let mut journey = state.journey.lock();
                                    // Add the length in seconds to the time now as the journey
                                    // length
                                    let time_out = Local::now()
                                        .checked_add_signed(Duration::seconds(time as i64))
                                        .unwrap();
                                    journey.push((time_out, path.clone()));

                                    // Add the weights to the graph
                                    for nodes in path.as_slice().windows(2) {
                                        // We know these are valid paths else A* would not have worked
                                        let mut edge =
                                            paths.edge_weight_mut(nodes[0], nodes[1]).unwrap();
                                        // Increase the crowdedness
                                        *edge = *edge + CAR;
                                    }

                                    // Send back the path as strings for the user
                                    let path = path.into_iter()
                                        .map(|i| {
                                            // TODO Make this abstraction sound and not leaky
                                            // Did this to get around some weird conversion trait
                                            // bounds but this should be handled properly at some point
                                            match i {
                                                0 => "New York City",
                                                1 => "Boston",
                                                2 => "Providence",
                                                3 => "Bar Harbor",
                                                _ => "DNE",
                                            }
                                        })
                                        .collect::<Vec<&str>>();

                                    // Set the body of the response
                                    match rpc::Response::new(
                                        json!({ "distance": time, "path": path }),
                                        req_json.id,
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
                                    // Send back an error message if invalid cities are used
                                    match rpc::Response::new(
                                        json!("Invalid start or end city given"),
                                        req_json.id,
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
                            // TODO clean up the `if let else` statements with better abstractions
                            } else {
                                ret.status(StatusCode::BAD_REQUEST);
                            }
                        } else {
                            ret.status(StatusCode::BAD_REQUEST);
                        }
                    } else {
                        ret.status(StatusCode::BAD_REQUEST);
                    }
                }
                _ => {
                    ret.status(StatusCode::NOT_FOUND);
                }
            }
        } else {
            ret.status(StatusCode::NOT_FOUND);
        }

        // Return the body
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
                (start, start + a.len())
            };

            let mut body_len = 0;

            for (i, header) in r.headers.iter().enumerate() {
                if header.name == "Content-Length" {
                    // We know Content-Length will always be a number and valid utf8 so this is
                    // okay. This assumes that no one sends a malformed payload however.
                    body_len = unsafe { str::from_utf8_unchecked(header.value) }
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
                body_len,
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
