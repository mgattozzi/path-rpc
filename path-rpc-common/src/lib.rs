//! Common data structures and functionality shared between the Client and the Server are stored
//! here. The Request and Response types are json-rpc 2.0 which means you can have non Rust based
//! clients also interact with the server!

// We need macros for the tests but rustc doesn't know that so we allow them for this import
#[allow(unused_imports)]
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate failure;
extern crate serde;

use serde_json::Value;
use std::io;

#[derive(Serialize, Deserialize, Debug, Clone)]
/// Request struct used to represent various requests that can be made to the server
/// Follows the json-rpc 2.0 spec
pub struct Request {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
    pub id: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
/// Request struct used to represent various responses that can be made to the server
/// Follows the json-rpc 2.0 spec
pub struct Response {
    pub jsonrpc: String,
    pub result: Value,
    pub id: u64,
}

impl Request {
    /// Constructs a new Request for the server. The `json!()` macro from serde_json should be
    /// utilized to construct the params field easily, rather than manually done.
    ///
    /// ## Example
    ///
    /// ```rust
    /// #[macro_use] extern crate serde_json;
    /// extern crate path_rpc_common;
    ///
    /// use serde_json::Value;
    /// use path_rpc_common::Request;
    ///
    /// let req = Request::new("new_path", json!({ "test": "testing" }), 0);
    /// let mut map = serde_json::map::Map::new();
    /// map.insert("test".into(), Value::String("testing".into()));
    /// let val = Value::Object(map);
    ///
    /// assert_eq!(req.jsonrpc, "2.0");
    /// assert_eq!(req.method, "new_path");
    /// assert_eq!(req.params, val);
    /// assert_eq!(req.id, 0);
    /// ```
    ///
    pub fn new(method: &str, params: Value, id: u64) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id,
        }
    }

    /// Turn the `Request` into json to be sent to the server
    ///
    /// ## Example
    ///
    /// ```rust
    /// #[macro_use] extern crate serde_json;
    /// extern crate path_rpc_common;
    ///
    /// use path_rpc_common::Request;
    ///
    /// let req = Request::new("new_path", json!({ "test": "testing" }), 0);
    /// let serialized = req.to_json().unwrap();
    /// let clone = Request::from_json(&serialized).unwrap();
    ///
    /// assert_eq!(req.jsonrpc, clone.jsonrpc);
    /// assert_eq!(req.method, clone.method);
    /// assert_eq!(req.params, clone.params);
    /// assert_eq!(req.id, clone.id);
    /// ```
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string(&self).map_err(|e| Error::from(e))
    }

    /// Turn the json into a `Request` that was sent to the server
    ///
    /// ## Example
    ///
    /// ```rust
    /// #[macro_use] extern crate serde_json;
    /// extern crate path_rpc_common;
    ///
    /// use path_rpc_common::Request;
    ///
    /// let req = Request::new("new_path", json!({ "test": "testing" }), 0);
    /// let serialized = req.to_json().unwrap();
    /// let clone = Request::from_json(&serialized).unwrap();
    ///
    /// assert_eq!(req.jsonrpc, clone.jsonrpc);
    /// assert_eq!(req.method, clone.method);
    /// assert_eq!(req.params, clone.params);
    /// assert_eq!(req.id, clone.id);
    /// ```
    pub fn from_json(json: &str) -> Result<Request, Error> {
        serde_json::from_str(json).map_err(|e| Error::from(e))
    }
}

impl Response {
    /// Constructs a new Response for the server. The `json!()` macro from serde_json should be
    /// utilized to construct the result field easily, rather than manually done.
    ///
    /// ## Example
    ///
    /// ```rust
    /// #[macro_use] extern crate serde_json;
    /// extern crate path_rpc_common;
    ///
    /// use serde_json::Value;
    /// use path_rpc_common::Response;
    ///
    /// let res = Response::new(json!("Bad Request"), 0);
    ///
    /// assert_eq!(res.jsonrpc, "2.0");
    /// assert_eq!(res.result, Value::String("Bad Request".into()));
    /// assert_eq!(res.id, 0);
    /// ```
    ///
    pub fn new(result: Value, id: u64) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            result,
            id,
        }
    }

    /// Turn the `Response` into json to be sent from the server
    ///
    /// ## Example
    ///
    /// ```rust
    /// #[macro_use] extern crate serde_json;
    /// extern crate path_rpc_common;
    /// use path_rpc_common::Response;
    ///
    /// let res = Response::new(json!("Bad Request"), 0);
    /// let serialized = res.to_json().unwrap();
    /// let clone = Response::from_json(&serialized).unwrap();
    ///
    /// assert_eq!(res.id, clone.id);
    /// assert_eq!(res.result, clone.result);
    /// assert_eq!(res.jsonrpc, clone.jsonrpc);
    /// ```
    pub fn to_json(&self) -> Result<String, Error> {
        serde_json::to_string(&self).map_err(|e| Error::from(e))
    }

    /// Turn the json into a `Response` that was sent from the server
    ///
    /// ## Example
    ///
    /// ```rust
    /// #[macro_use] extern crate serde_json;
    /// extern crate path_rpc_common;
    /// use path_rpc_common::Response;
    ///
    /// let res = Response::new(json!("Bad Request"), 0);
    /// let serialized = res.to_json().unwrap();
    /// let clone = Response::from_json(&serialized).unwrap();
    ///
    /// assert_eq!(res.id, clone.id);
    /// assert_eq!(res.result, clone.result);
    /// assert_eq!(res.jsonrpc, clone.jsonrpc);
    /// ```
    pub fn from_json(json: &str) -> Result<Response, Error> {
        serde_json::from_str(json).map_err(|e| Error::from(e))
    }
}

#[derive(Debug, Fail)]
pub enum Error {
    /// Maps any underlying I/O errors that are thrown to this variant
    #[fail(display = "{}", _0)]
    Io(#[cause] io::Error),
    #[fail(display = "{}", _0)]
    SerdeJson(#[cause] serde_json::Error),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::SerdeJson(e)
    }
}
