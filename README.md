# path-rpc

## About
This demonstrates a tokio service that can be asked for the paths between
cities and adds crowdedness with each request. As the cars "complete" their
journey they're removed from the graph and the speed delay they add is removed.
Each request gets the optimal path at that point in time using the A* algorithm.

## Example

Here's how to get the example up and running:

To get the server running:

```bash
$ cd path-rpc-server && cargo run
```
when it's up you should see:

```bash
Listening on: 0.0.0.0:8080
```

In a separate terminal run the following:

```bash
$ cd path-rpc-client && cargo run --example client
```

You should see that it prints out values as such:

```bash
Distance: 31
Path: ["Boston", "New York City"]
```

You should see it print out multiple of these from the same path request (see
`path-rpc-client/examples/client.rs`) and that the distance changes. To see that
the numbers go up under high stress, run the client a few times. After you wait
for a bit and run it again you should see the numbers have settled down.

## Design

### Server
The service sets up workers for a number of threads equal to the amount the CPU
has minus one, where the last one is used to garbage collect the journeys and
paths to keep track of the graph and it's values. It includes a very basic
attempt at HTTP parsing to show off what tokio can do, but is not reccomended
for every day http requests, and something like Hyper is better suited to that.

There's a global state representing the graph and the journey's currently
occuring that can be referenced. They're locked behind mutices in order to make
sure that nothing is modified out of order.

The graph needs to use `u64` as identifiers due to `Copy` bound restrictions,
making it hard to make a bigger graph, without manually putting a lot of the
items in oneself. This leads to a somewhat leaky abstraction between names of
cities and the number that represents it. More work could be done here to make
more compile time guarantees.

The code is a highly modified version of `tokio-core`'s
[tinyhttp](https://github.com/tokio-rs/tokio-core/blob/master/examples/tinyhttp.rs) example, as
it didn't have the ability to parse the body of a request, nor did it handle the
rpc request/responses we needed for the service, nor the logic to get that
working.

### Common
This contains types needed for both and to reduce repetition, mainly, a Response
and a Request type in the form of json-rpc Version 2.0. This makes it easy to
set a standard for non rust clients to interact with the server as well as
standardizing input and output of the Server and Client

### Client
The client was designed using hyper/tokio-core/futures to make things cleaner with the API.
There is only one type of request now but it could easily be expanded to include
new types of requests to the server. It works by using a function call to
generate a future for the type of request that one wants to make. This then
allows the end user to modify what else they want the future to do. They can
then call `run` on it in order to execute the future to completion.

It's fairly simple in terms of design, but allows flexibility for the end user
to do what they want with the response from the server.
