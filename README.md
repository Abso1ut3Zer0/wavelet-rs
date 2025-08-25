# Notes

- Have a reactor node with an embedded mio::Poller to handle events efficiently
- Make sure to have async capabilities within a node
    -- Node should be able to poll a future that we assign to the node
    -- Interacts with the reactor node to register pollable events
- Ensure that we can work across thread bounds, to work with background threads
    -- Use mio Event (event fd wrappers) to manage this?
- Nodes should be wrappers around parents, and procedures on parents.
    -- Make sure we can also track mutable state
    -- Ideally it should feel less like we are working with nodes and more with procedures
- Graphs should be thread local, so we call into the graph with just function calls
- Data on the node and the procuder need to be separate, i.e. not a trait method
    -- Have a Box<dyn FnMut(ctx: &GraphContext) -> bool)> closure
    -- Associate this closure with the node in a petgraph directed graph
    -- Petgraph node should have a weak reference to the mutable state data
