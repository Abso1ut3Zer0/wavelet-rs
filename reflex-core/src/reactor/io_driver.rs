use petgraph::prelude::NodeIndex;

pub struct IoHandle(mio::Token);

#[derive(Debug, Clone, Copy)]
pub(super) struct IoRegistration {
    node_index: NodeIndex,
    interest: mio::Interest,
}

pub struct IoDriver {}
