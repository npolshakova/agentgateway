//! Gateway dynamic CA support.
//!
//! Generates and caches per-hostname leaf certificates signed by a configured
//! dynamic CA so agentgateway can terminate TLS using a certificate that matches
//! the downstream client's SNI.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use rcgen::{CertificateParams, DnType, Issuer, KeyPair};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use rustls::server::ResolvesServerCert;
use rustls::sign::CertifiedKey;

use crate::transport::tls;
use crate::types::agent::{ServerTLSConfig, TLSVersion};

struct DynamicCa {
	cert_der: Vec<u8>,
	issuer: Issuer<'static, KeyPair>,
}

impl DynamicCa {
	fn from_pem(cert_pem: &[u8], key_pem: &[u8]) -> anyhow::Result<Self> {
		let cert_pem_str = std::str::from_utf8(cert_pem)?;
		let key_pem_str = std::str::from_utf8(key_pem)?;

		let cert_der = CertificateDer::pem_slice_iter(cert_pem)
			.next()
			.ok_or_else(|| anyhow!("no certificate found in dynamic CA PEM"))?
			.map_err(|e| anyhow!("failed to parse dynamic CA cert PEM: {e}"))?
			.to_vec();

		let key_pair = KeyPair::from_pem(key_pem_str)?;
		let issuer = Issuer::from_ca_cert_pem(cert_pem_str, key_pair)?;

		Ok(Self { cert_der, issuer })
	}

	fn generate_leaf_cert(&self, domain: &str) -> anyhow::Result<(Vec<u8>, Vec<u8>)> {
		let mut params = CertificateParams::new(vec![domain.to_string()])?;
		params.distinguished_name.push(DnType::CommonName, domain);
		params.is_ca = rcgen::IsCa::NoCa;
		params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
		params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];

		let leaf_key = KeyPair::generate()?;
		let leaf_cert = params.signed_by(&leaf_key, &self.issuer)?;

		Ok((leaf_cert.der().to_vec(), leaf_key.serialize_der()))
	}
}

struct CachedDynamicCaCert {
	certified_key: Arc<CertifiedKey>,
	issued_at: Instant,
}

#[derive(Default)]
struct DynamicCaCertCache {
	entries: HashMap<String, CachedDynamicCaCert>,
	order: VecDeque<String>,
}

struct DynamicCaCertResolver {
	ca: Arc<DynamicCa>,
	provider: Arc<rustls::crypto::CryptoProvider>,
	cache: Mutex<DynamicCaCertCache>,
	cache_ttl: Duration,
	cache_capacity: usize,
}

impl std::fmt::Debug for DynamicCaCertResolver {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("DynamicCaCertResolver").finish()
	}
}

impl DynamicCaCertResolver {
	fn lock_cache(&self) -> MutexGuard<'_, DynamicCaCertCache> {
		self
			.cache
			.lock()
			.unwrap_or_else(|poisoned| poisoned.into_inner())
	}

	fn cached_fresh_entry(
		cache: &DynamicCaCertCache,
		domain: &str,
		now: Instant,
		cache_ttl: Duration,
	) -> Option<Arc<CertifiedKey>> {
		let entry = cache.entries.get(domain)?;
		if now.duration_since(entry.issued_at) <= cache_ttl {
			return Some(Arc::clone(&entry.certified_key));
		}
		None
	}

	fn generate_certified_key(&self, domain: &str) -> Option<Arc<CertifiedKey>> {
		let (leaf_der, key_der) = self.ca.generate_leaf_cert(domain).ok()?;

		let cert_chain = vec![
			CertificateDer::from(leaf_der),
			CertificateDer::from(self.ca.cert_der.clone()),
		];

		let private_key = PrivatePkcs8KeyDer::from(key_der);
		let signing_key = self
			.provider
			.key_provider
			.load_private_key(private_key.into())
			.ok()?;

		Some(Arc::new(CertifiedKey::new(cert_chain, signing_key)))
	}

	fn cached_certified_key(&self, domain: &str) -> Option<Arc<CertifiedKey>> {
		{
			let now = Instant::now();
			let mut cache = self.lock_cache();

			if let Some(certified_key) = Self::cached_fresh_entry(&cache, domain, now, self.cache_ttl) {
				return Some(certified_key);
			}

			cache.entries.remove(domain);
			cache.order.retain(|cached| cached != domain);
		}

		let certified_key = self.generate_certified_key(domain)?;

		let now = Instant::now();
		let mut cache = self.lock_cache();
		if let Some(existing) = Self::cached_fresh_entry(&cache, domain, now, self.cache_ttl) {
			return Some(existing);
		}

		cache.entries.remove(domain);
		cache.order.retain(|cached| cached != domain);
		cache.entries.insert(
			domain.to_string(),
			CachedDynamicCaCert {
				certified_key: Arc::clone(&certified_key),
				issued_at: now,
			},
		);
		cache.order.push_back(domain.to_string());

		while cache.order.len() > self.cache_capacity {
			if let Some(oldest) = cache.order.pop_front() {
				cache.entries.remove(&oldest);
			}
		}

		Some(certified_key)
	}
}

impl ResolvesServerCert for DynamicCaCertResolver {
	fn resolve(&self, client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
		let domain = client_hello.server_name()?;
		self.cached_certified_key(domain)
	}
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_dynamic_ca_server_config(
	ca_cert_pem: &[u8],
	ca_key_pem: &[u8],
	alpns: Option<&[Vec<u8>]>,
	default_alpns: &[Vec<u8>],
	min_version: Option<TLSVersion>,
	max_version: Option<TLSVersion>,
	cipher_suites: &[tls::CipherSuite],
	key_exchange_groups: &[tls::KeyExchangeGroup],
	cache_config: &crate::DynamicCaCertCacheConfig,
) -> anyhow::Result<rustls::ServerConfig> {
	let provider = tls::provider_with_options(cipher_suites, key_exchange_groups);

	let versions = super::agent::tls_versions_for_range(min_version, max_version)?;
	let mut config = rustls::ServerConfig::builder_with_provider(Arc::clone(&provider))
		.with_protocol_versions(&versions)
		.expect("server config must be valid")
		.with_no_client_auth()
		.with_cert_resolver(Arc::new(DynamicCaCertResolver {
			ca: Arc::new(DynamicCa::from_pem(ca_cert_pem, ca_key_pem)?),
			provider,
			cache: Mutex::new(DynamicCaCertCache::default()),
			cache_ttl: cache_config.ttl,
			cache_capacity: cache_config.capacity,
		}));
	config.key_log = tls::key_log();
	config.alpn_protocols = alpns
		.map(|a| a.to_vec())
		.unwrap_or_else(|| default_alpns.to_vec());

	Ok(config)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_dynamic_ca_tls_config_with_profile(
	ca_cert_pem: Vec<u8>,
	ca_key_pem: Vec<u8>,
	default_alpns: Vec<Vec<u8>>,
	min_version: Option<TLSVersion>,
	max_version: Option<TLSVersion>,
	cipher_suites: Option<Vec<tls::CipherSuite>>,
	key_exchange_groups: Option<Vec<tls::KeyExchangeGroup>>,
	cache_config: crate::DynamicCaCertCacheConfig,
) -> anyhow::Result<ServerTLSConfig> {
	ServerTLSConfig::dynamic_ca_with_profile(
		ca_cert_pem,
		ca_key_pem,
		default_alpns,
		min_version,
		max_version,
		cipher_suites,
		key_exchange_groups,
		cache_config,
	)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn test_resolver() -> DynamicCaCertResolver {
		let ca_key = rcgen::KeyPair::generate().expect("generate CA key");
		let mut ca_params = rcgen::CertificateParams::default();
		ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
		let ca_cert = ca_params.self_signed(&ca_key).expect("generate CA cert");

		DynamicCaCertResolver {
			ca: Arc::new(
				DynamicCa::from_pem(ca_cert.pem().as_bytes(), ca_key.serialize_pem().as_bytes())
					.expect("parse CA"),
			),
			provider: tls::provider_with_options(&[], &[]),
			cache: Mutex::new(DynamicCaCertCache::default()),
			cache_ttl: crate::DynamicCaCertCacheConfig::default().ttl,
			cache_capacity: crate::DynamicCaCertCacheConfig::default().capacity,
		}
	}

	#[test]
	fn cached_certified_key_reuses_fresh_entry() {
		let resolver = test_resolver();

		let first = resolver
			.cached_certified_key("example.com")
			.expect("generate cert");
		let second = resolver
			.cached_certified_key("example.com")
			.expect("cache hit");

		assert!(Arc::ptr_eq(&first, &second));
	}

	#[test]
	fn cached_certified_key_recovers_from_poisoned_cache_lock() {
		let resolver = test_resolver();
		let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			let _cache = resolver.cache.lock().expect("lock cache");
			panic!("poison dynamic CA cache lock");
		}));

		assert!(resolver.cached_certified_key("example.com").is_some());
	}
}
