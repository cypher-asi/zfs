#![forbid(unsafe_code)]

mod context;
mod descriptor;
mod error;
mod gossip;
mod identity;
mod registry;
mod service;
mod wasm_bindings;
mod wasm_host;

pub use context::{ProgramStore, ServiceContext, ServiceEvent, TopicCommand};
pub use descriptor::{ServiceDescriptor, ServiceId};
pub use error::ServiceError;
pub use gossip::ServiceGossipHandler;
pub use identity::NodeIdentity;
pub use registry::{ServiceInfo, ServiceRegistry};
pub use service::{RouteInfo, Service};
pub use wasm_host::{
    load_descriptor as load_wasm_descriptor, WasmResourceLimits, WasmServiceConfig,
};
