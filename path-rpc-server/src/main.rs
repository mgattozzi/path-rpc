extern crate petgraph;
extern crate failure;
extern crate antidote;

use petgraph::graphmap::DiGraphMap;
use petgraph::visit::EdgeRef;
use petgraph::algo::astar;
use failure::Error;
use std::sync::Arc;
use antidote::Mutex;

fn main() -> Result<(), Error> {
    let state = State::new();
    let g = state.paths.lock();
    let path = astar(&*g, "Bar Harbor", |finish| finish == "New York City", |e| *e.weight(), |_| 0);

    if let Some(i) = path {
        println!("Distance: {}", i.0);
        println!("path: {:?}", i.1);
    } else {
        println!("Invalid Path Given");
    }
    Ok(())
}

#[derive(Clone)]
struct State {
    paths: Arc<Mutex<DiGraphMap<&'static str, u64>>>
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
            paths: Arc::new(Mutex::new(map))
        }
    }
}
