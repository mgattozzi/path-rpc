extern crate antidote;
extern crate futures;
extern crate hyper;
#[macro_use]
extern crate failure;
extern crate tokio_core;
#[macro_use]
extern crate serde_json;
extern crate path_rpc_common as rpc;

use antidote::Mutex;
use futures::future::Future;
use futures::Stream;
use hyper::client::HttpConnector;
use hyper::header::{ContentLength, ContentType};
use hyper::Client;
use hyper::Method;
use hyper::Request;
use hyper::Uri;
use tokio_core::reactor::Core;

pub use failure::Error;
pub use rpc::Response;

use std::str;
use std::sync::Arc;

unsafe impl Send for PathPlanner {}
unsafe impl Sync for PathPlanner {}

/// A client that will send requests to a path-rpc-server and can be used across threads
pub struct PathPlanner {
    core: Arc<Mutex<Core>>,
    client: Arc<Client<HttpConnector>>,
    url: Arc<Mutex<String>>,
}

impl PathPlanner {
    /// Create a new client that will send requests to a path-rpc-server
    pub fn new(url: &str) -> Result<Self, Error> {
        let core = Core::new()?;
        let client = Client::new(&core.handle());

        Ok(Self {
            core: Arc::new(Mutex::new(core)),
            client: Arc::new(client),
            url: Arc::new(Mutex::new(url.to_string())),
        })
    }

    /// Given a start and an end as well as an id for the transaction create a Future that will
    /// deserialize the response into an `rpc::Response`. Note this doesn't actually execute the
    /// `Future`. This allows the consumer to chain additional methods to the computation they wish
    /// to perform. To execute the `Future` it will need to call `PathPlanner`'s run function to
    /// get a result.
    pub fn get_path(
        &self,
        start: &str,
        end: &str,
        id: u64,
    ) -> Result<impl Future<Item = Result<Response, Error>>, Error> {
        Ok(self.client
            // Setup and execute the request
            .request({
                let lock = self.url.lock();
                let mut url = lock.clone();
                url.push_str("/path");
                let mut req = Request::new(Method::Post, url.parse::<Uri>()?);
                let body = rpc::Request::new("get_path", json!({"start": start, "end": end}), id)
                    .to_json()?;
                {
                    let headers = req.headers_mut();
                    headers.set(ContentType::json());
                    headers.set(ContentLength(body.len() as u64));
                }
                req.set_body(body);
                req
            })
            // Turn the chunks of the body into the Response type or an error
            .and_then(|res| {
                res.body().concat2().map(move |chunks| {
                    if chunks.is_empty() {
                        bail!("Empty body returned")
                    } else {
                        Ok(rpc::Response::from_json(str::from_utf8(&chunks)?)?)
                    }
                })
            }))
    }

    /// Run any future given to the client
    pub fn run<T: Future>(&self, future: T) -> Result<T::Item, T::Error> {
        let mut lock = self.core.lock();
        lock.run(future)
    }
}
