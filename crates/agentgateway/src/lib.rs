// For now, the entire package is not linked up to anything so squash the warnings
#![allow(unused)]
use std::collections::BTreeMap;
use std::fmt::{Debug, Display};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::{fmt, io};

use agent_core::prelude::*;
use control::caclient::CaClient;
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use indexmap::IndexMap;
#[cfg(feature = "schema")]
pub use schemars::JsonSchema;
use serde::de::Visitor;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
pub use serdes::*;

use crate::store::Stores;
use crate::types::discovery::Identity;

pub mod a2a;
pub mod app;
pub mod cel;
pub mod client;
pub mod config;
pub mod control;
pub mod http;
pub mod json;
pub mod llm;
pub mod management;
pub mod mcp;
pub mod parse;
pub mod proxy;
pub mod serdes;
pub mod state_manager;
pub mod store;
mod telemetry;
pub mod transport;
pub mod types;
#[cfg(feature = "ui")]
mod ui;
pub mod util;

use agent_core::prelude::*;
use control::caclient;
use telemetry::{metrics, trc};

use crate::telemetry::trc::Protocol;

#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
/// NestedRawConfig represents a subset of the config that can be passed in. This is split out from static
/// and dynamic config
pub struct NestedRawConfig {
	config: Option<RawConfig>,
}

#[derive(serde::Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
// RawConfig represents the inputs a user can pass in. Config represents the internal representation of this.
pub struct RawConfig {
	enable_ipv6: Option<bool>,

	/// Local XDS path. If not specified, the current configuration file will be used.
	local_xds_path: Option<PathBuf>,

	ca_address: Option<String>,
	xds_address: Option<String>,
	namespace: Option<String>,
	gateway: Option<String>,
	trust_domain: Option<String>,
	service_account: Option<String>,
	cluster_id: Option<String>,
	network: Option<String>,

	/// Admin UI address in the format "ip:port"
	admin_addr: Option<String>,
	/// Stats/metrics server address in the format "ip:port"
	stats_addr: Option<String>,
	/// Readiness probe server address in the format "ip:port"
	readiness_addr: Option<String>,

	auth_token: Option<String>,

	#[serde(default, with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	connection_termination_deadline: Option<Duration>,
	#[serde(default, with = "serde_dur_option")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	connection_min_termination_deadline: Option<Duration>,

	#[cfg_attr(feature = "schema", schemars(with = "String"))]
	worker_threads: Option<StringOrInt>,

	tracing: Option<RawTracing>,
	logging: Option<RawLogging>,

	http2: Option<RawHTTP2>,
}

#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct RawHTTP2 {
	window_size: Option<u32>,
	connection_window_size: Option<u32>,
	frame_size: Option<u32>,
	pool_max_streams_per_conn: Option<u16>,
	#[serde(deserialize_with = "serde_dur_option::deserialize")]
	#[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
	pool_unused_release_timeout: Option<Duration>,
}

#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct RawTracing {
	otlp_endpoint: String,
	#[serde(default)]
	otlp_protocol: Protocol,
	fields: Option<RawLoggingFields>,
}

#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct RawLogging {
	filter: Option<String>,
	fields: Option<RawLoggingFields>,
}

#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct RawLoggingFields {
	#[serde(default)]
	remove: Vec<String>,
	#[serde(default)]
	#[cfg_attr(
		feature = "schema",
		schemars(with = "std::collections::HashMap<String, String>")
	)]
	add: IndexMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct StringOrInt(String);

impl<'de> Deserialize<'de> for StringOrInt {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		struct StringOrIntVisitor();

		impl Visitor<'_> for StringOrIntVisitor {
			type Value = StringOrInt;

			fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
				formatter.write_str("string or int")
			}

			fn visit_str<E>(self, value: &str) -> Result<StringOrInt, E>
			where
				E: de::Error,
			{
				Ok(StringOrInt(value.to_owned()))
			}

			fn visit_i64<E>(self, value: i64) -> Result<StringOrInt, E> {
				Ok(StringOrInt(value.to_string()))
			}
		}

		deserializer.deserialize_any(StringOrIntVisitor())
	}
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
	pub network: Strng,
	#[serde(with = "serde_dur")]
	pub termination_max_deadline: Duration,
	#[serde(with = "serde_dur")]
	pub termination_min_deadline: Duration,
	/// Specify the number of worker threads the Tokio Runtime will use.
	pub num_worker_threads: usize,
	pub admin_addr: Address,
	pub stats_addr: Address,
	pub readiness_addr: Address,
	// For waypoint identification
	pub self_addr: Option<Strng>,
	pub hbone: Arc<agent_hbone::Config>,
	/// XDS address to use. If unset, XDS will not be used.
	pub xds: XDSConfig,
	pub ca: Option<caclient::Config>,
	pub tracing: trc::Config,
	pub logging: crate::telemetry::log::Config,
	pub dns: client::Config,
	pub proxy_metadata: ProxyMetadata,
	pub threading_mode: ThreadingMode,
}

#[derive(serde::Serialize, Copy, PartialOrd, PartialEq, Eq, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum ThreadingMode {
	#[default]
	Multithreaded,
	// Experimental; do not use beyond testing
	ThreadPerCore,
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct XDSConfig {
	/// XDS address to use. If unset, XDS will not be used.
	pub address: Option<String>,
	pub namespace: String,
	pub gateway: String,

	pub local_config: Option<ConfigSource>,
}

#[derive(Clone, Debug)]
pub enum ConfigSource {
	File(PathBuf),
	Static(Bytes),
	// #[cfg(any(test, feature = "testing"))]
	// Dynamic(Arc<tokio::sync::Mutex<MpscAckReceiver<LocalConfig>>>),
}

impl Serialize for ConfigSource {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		match self {
			ConfigSource::File(name) => serializer.serialize_str(&name.to_string_lossy()),
			ConfigSource::Static(_) => serializer.serialize_str("static"),
		}
	}
}

impl ConfigSource {
	pub async fn read_to_string(&self) -> anyhow::Result<String> {
		Ok(match self {
			ConfigSource::File(path) => fs_err::tokio::read_to_string(path).await?,
			ConfigSource::Static(data) => std::str::from_utf8(data).map(|s| s.to_string())?,
			// #[cfg(any(test, feature = "testing"))]
			// _ => "{}".to_string(),
		})
	}
	pub fn read_to_string_sync(&self) -> anyhow::Result<String> {
		Ok(match self {
			ConfigSource::File(path) => fs_err::read_to_string(path)?,
			ConfigSource::Static(data) => std::str::from_utf8(data).map(|s| s.to_string())?,
			// #[cfg(any(test, feature = "testing"))]
			// _ => "{}".to_string(),
		})
	}
}

#[derive(Debug, Clone)]
pub struct ProxyInputs {
	cfg: Arc<Config>,
	stores: Stores,

	upstream: client::Client,

	metrics: Arc<metrics::Metrics>,
	tracer: Option<trc::Tracer>,

	mcp_state: mcp::sse::App,
	ca: Option<Arc<CaClient>>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
// Address is a wrapper around either a normal SocketAddr or "bind to localhost on IPv4 and IPv6"
pub enum Address {
	// Bind to localhost (dual stack) on a specific port
	// (ipv6_enabled, port)
	Localhost(bool, u16),
	// Bind to an explicit IP/port
	SocketAddr(SocketAddr),
}

impl Display for Address {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Address::Localhost(_, port) => write!(f, "localhost:{port}"),
			Address::SocketAddr(s) => write!(f, "{s}"),
		}
	}
}

impl IntoIterator for Address {
	type Item = SocketAddr;
	type IntoIter = <Vec<std::net::SocketAddr> as IntoIterator>::IntoIter;

	fn into_iter(self) -> Self::IntoIter {
		match self {
			Address::Localhost(ipv6_enabled, port) => {
				if ipv6_enabled {
					vec![
						SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
						SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port),
					]
					.into_iter()
				} else {
					vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)].into_iter()
				}
			},
			Address::SocketAddr(s) => vec![s].into_iter(),
		}
	}
}

impl Address {
	fn new(ipv6_enabled: bool, s: &str) -> anyhow::Result<Self> {
		if s.starts_with("localhost:") {
			let (_host, ports) = s.split_once(':').expect("already checked it has a :");
			let port: u16 = ports.parse()?;
			Ok(Address::Localhost(ipv6_enabled, port))
		} else {
			Ok(Address::SocketAddr(s.parse()?))
		}
	}

	pub fn port(&self) -> u16 {
		match self {
			Address::Localhost(_, port) => *port,
			Address::SocketAddr(s) => s.port(),
		}
	}

	// with_ipv6 unconditionally overrides the IPv6 setting for the address
	pub fn with_ipv6(self, ipv6: bool) -> Self {
		match self {
			Address::Localhost(_, port) => Address::Localhost(ipv6, port),
			x => x,
		}
	}

	// maybe_downgrade_ipv6 updates the V6 setting, ONLY if the address was already V6
	pub fn maybe_downgrade_ipv6(self, updated_v6: bool) -> Self {
		match self {
			Address::Localhost(true, port) => Address::Localhost(updated_v6, port),
			x => x,
		}
	}
}

const IPV6_DISABLED_LO: &str = "/proc/sys/net/ipv6/conf/lo/disable_ipv6";

fn read_sysctl(key: &str) -> io::Result<String> {
	let mut file = File::open(key)?;
	let mut data = String::new();
	file.read_to_string(&mut data)?;
	Ok(data.trim().to_string())
}

pub fn ipv6_enabled_on_localhost() -> io::Result<bool> {
	read_sysctl(IPV6_DISABLED_LO).map(|s| s != "1")
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProxyMetadata {
	pub instance_ip: String,
	pub pod_name: String,
	pub pod_namespace: String,
	pub node_name: String,
	pub role: String,
	pub node_id: String,
}
