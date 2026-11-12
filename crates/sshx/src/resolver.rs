//! DNS resolution for the sshx client, with a fallback for Android/Termux.
//!
//! The Linux binaries of sshx are statically linked against musl, whose
//! resolver reads `/etc/resolv.conf`. That file does not exist on Android,
//! where Termux instead keeps it at `$PREFIX/etc/resolv.conf`, so DNS lookups
//! fail with `EAI_AGAIN` ("failed to lookup address information: Try again").
//!
//! To work around this, we first try the system resolver, which behaves the
//! same as before on all other platforms. If it fails, we fall back to
//! querying the nameservers listed in `$PREFIX/etc/resolv.conf` directly.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use hickory_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};
use hickory_resolver::name_server::TokioConnectionProvider;
use hickory_resolver::AsyncResolver;
use hyper_util::client::legacy::connect::dns::Name;
use tokio::sync::OnceCell;
use tower_service::Service;
use tracing::warn;

/// Default installation prefix of Termux, an Android terminal emulator, which
/// does not have a standard `/etc` directory on Android.
const TERMUX_PREFIX: &str = "/data/data/com.termux/files/usr";

type Fallback = Option<AsyncResolver<TokioConnectionProvider>>;

/// A DNS resolver that falls back to Termux's nameservers if the system
/// resolver is unable to look up an address.
///
/// This is a drop-in replacement for hyper-util's default `GaiResolver` that
/// keeps working on Android, where the system resolver is unavailable to
/// statically linked binaries.
#[derive(Clone, Default)]
pub struct FallbackResolver {
    /// Fallback nameservers to query, or `None` to read them from Termux's
    /// resolv.conf file lazily on first use.
    nameservers: Option<Vec<SocketAddr>>,
    /// Lazily initialized fallback resolver, shared between clones.
    fallback: Arc<OnceCell<Fallback>>,
}

impl FallbackResolver {
    /// Create a resolver that falls back to querying the given nameservers.
    pub fn with_nameservers(nameservers: Vec<SocketAddr>) -> Self {
        Self {
            nameservers: Some(nameservers),
            fallback: Arc::new(OnceCell::new()),
        }
    }

    /// Look up a host name, first with the system resolver, then with the
    /// fallback nameservers if that fails.
    async fn lookup(&self, host: &str) -> Result<Vec<SocketAddr>, std::io::Error> {
        // First, try the system resolver, matching the default behavior on
        // all other platforms. Failures are retried with the fallback
        // resolver below, in case we are running under Termux.
        let mut system = tokio::net::lookup_host((host, 0)).await;
        if let Ok(addrs) = &mut system {
            let addrs: Vec<_> = addrs.by_ref().collect();
            if !addrs.is_empty() {
                return Ok(addrs);
            }
        }

        // The system resolver failed, so query the fallback nameservers.
        let nameservers = self.nameservers.clone();
        let resolver = self
            .fallback
            .get_or_init(|| async {
                let nameservers = match nameservers {
                    Some(nameservers) => nameservers,
                    None => match termux_nameservers().await {
                        Ok(nameservers) => nameservers,
                        Err(err) => {
                            warn!(%err, "could not read Termux resolv.conf");
                            vec![]
                        }
                    },
                };
                init_fallback(&nameservers)
            })
            .await;
        let Some(resolver) = resolver else {
            return match system {
                // No fallback is available, so report the original error
                // from the system resolver for better diagnostics.
                Err(err) => Err(err),
                Ok(_) => Err(std::io::Error::other("DNS lookup returned no addresses")),
            };
        };

        match resolver.lookup_ip(host).await {
            Ok(lookup) => {
                let addrs: Vec<_> = lookup.iter().map(|ip| SocketAddr::new(ip, 0)).collect();
                if addrs.is_empty() {
                    Err(std::io::Error::other("DNS lookup returned no addresses"))
                } else {
                    Ok(addrs)
                }
            }
            Err(err) => Err(std::io::Error::other(format!(
                "fallback DNS resolution failed: {err}"
            ))),
        }
    }
}

impl Service<Name> for FallbackResolver {
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    type Response = std::vec::IntoIter<SocketAddr>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let this = self.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            // hyper-util overrides the port of the returned addresses, so the
            // port is irrelevant here and is set to zero.
            let addrs = this.lookup(&host).await?;
            Ok(addrs.into_iter())
        })
    }
}

/// Build a fallback resolver for the given nameservers, or `None` if there
/// are none to query.
fn init_fallback(nameservers: &[SocketAddr]) -> Fallback {
    if nameservers.is_empty() {
        return None;
    }
    let ips: Vec<IpAddr> = nameservers.iter().map(SocketAddr::ip).collect();
    let port = nameservers[0].port();
    let group = NameServerConfigGroup::from_ips_clear(&ips, port, true);
    let config = ResolverConfig::from_parts(None, vec![], group);
    let mut options = ResolverOpts::default();
    options.attempts = 1; // the controller retries connections on failure
    Some(AsyncResolver::new(
        config,
        options,
        TokioConnectionProvider::default(),
    ))
}

/// Read the nameservers from Termux's `$PREFIX/etc/resolv.conf` file.
async fn termux_nameservers() -> std::io::Result<Vec<SocketAddr>> {
    let prefix = std::env::var("PREFIX").unwrap_or_else(|_| TERMUX_PREFIX.to_string());
    let contents = tokio::fs::read_to_string(format!("{prefix}/etc/resolv.conf")).await?;
    Ok(parse_nameservers(&contents)
        .into_iter()
        .map(|ip| SocketAddr::new(ip, 53))
        .collect())
}

/// Extract the IP addresses from the `nameserver` lines of a resolv.conf file.
fn parse_nameservers(contents: &str) -> Vec<IpAddr> {
    contents
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            match parts.next() {
                Some("nameserver") => parts.next()?.parse().ok(),
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nameservers_from_termux_config() {
        let contents = "\
# Generated by Termux
nameserver 8.8.8.8
nameserver 8.8.4.4
options timeout:1 attempts:1
";
        let ns = parse_nameservers(contents);
        let expected: Vec<IpAddr> = vec!["8.8.8.8".parse().unwrap(), "8.8.4.4".parse().unwrap()];
        assert_eq!(ns, expected);
    }

    #[test]
    fn parse_nameservers_ignores_other_lines() {
        assert!(parse_nameservers("").is_empty());
        assert!(parse_nameservers("search example.com\noptions ndots:2").is_empty());
        assert!(parse_nameservers("nameserver not-an-ip").is_empty());
    }

    #[test]
    fn parse_nameservers_accepts_ipv6() {
        let contents = "nameserver 2001:4860:4860::8888";
        let ns = parse_nameservers(contents);
        let expected: IpAddr = "2001:4860:4860::8888".parse().unwrap();
        assert_eq!(ns, vec![expected]);
    }

    #[tokio::test]
    async fn fallback_resolver_queries_system_dns_first() {
        // The system resolver should be able to look up a name that is
        // expected to exist, without using the fallback.
        use std::str::FromStr;
        let mut resolver = FallbackResolver::default();
        let name = Name::from_str("localhost").unwrap();
        let addrs: Vec<SocketAddr> = resolver.call(name).await.unwrap().collect();
        assert!(!addrs.is_empty());
    }
}
