//! This assumes you already have a `path-rpc-server` running locally on port 8080

extern crate futures;
extern crate path_rpc_client as rpc;

use std::error;
use std::io;

use futures::future::{ok, Future};
use rpc::{Error, PathPlanner, Response};

fn main() -> Result<(), Box<error::Error>> {
    let planner = PathPlanner::new("http://localhost:8080")?;

    let path1 = || work(planner.get_path("Boston", "New York City", 0).unwrap());
    let path2 = || work(planner.get_path("Bar Harbor", "New York City", 0).unwrap());

    planner.run(path2()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;

    // This is to show that the path changes over time as cars are added
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    planner.run(path1()).map_err(|_| err())?;
    Ok(())
}

fn err() -> io::Error {
    io::Error::new(io::ErrorKind::Other, "Execution failed!")
}

// We want to show off that we can further manipulate the future before we need to run it, leaving
// it up to the end user to do with the results of the response as they please
pub fn work(future: impl Future<Item = Result<Response, Error>>) -> impl Future {
    future.and_then(|f| {
        // We're making a lot of assumptions here that we get a response back shaped like we
        // expect, but we know for certain that bar some kind of error it will be.

        let value = f.unwrap();
        let distance = value.result.get("distance").unwrap();
        let path = value
            .result
            .get("path")
            .unwrap()
            .as_array()
            .unwrap()
            .into_iter()
            .map(|e| e.as_str().unwrap())
            .collect::<Vec<&str>>();
        println!("Distance: {}\nPath: {:?}\n", distance, path);
        ok(())
    })
}
